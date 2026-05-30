// core/src/runner/main.cpp — `prisma_run` CLI.
//
// Reads a guest x86_64 byte sequence (from a file path or stdin),
// translates it through the Prisma `Translator`, executes the
// resulting ARM64 code via the `Dispatcher`, and prints the final
// guest CPU state as JSON.
//
// The bytes can be either a "raw blob" (interpreted as starting at
// guest PC `--entry` / 0x4000 by default) or a tiny ELF (the harness
// in `tools/diff-qemu/` produces these). Today only raw blobs are
// supported; ELF parsing is the next slice.
//
// Usage:
//   prisma_run path/to/bytes.bin            # raw blob, entry 0x4000
//   prisma_run --entry 0x1000 bytes.bin     # custom entry PC
//   prisma_run --max-steps 100 bytes.bin    # cap dispatcher iterations
//   echo "C3" | prisma_run --hex -          # hex stdin: "ret"
//
// Exit codes:
//   0 = halted cleanly (program ended via Return).
//   1 = translation / dispatcher error.
//   2 = bad CLI arguments.

#include <cstdint>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <fstream>
#include <iostream>
#include <span>
#include <sstream>
#include <string>
#include <string_view>
#include <variant>
#include <vector>

#include "prisma/cpu_state.hpp"
#include "prisma/dispatcher.hpp"
#include "prisma/translator.hpp"

namespace {

struct Options {
    std::string   path;
    std::uint64_t entry_pc{0x4000};
    std::size_t   max_steps{1000};
    bool          hex_input{false};
    bool          verbose{false};
};

void print_usage() {
    std::fprintf(stderr,
        "prisma_run — translate + execute a raw x86_64 byte blob.\n"
        "\n"
        "usage: prisma_run [--entry HEX] [--max-steps N] [--hex] [--verbose]\n"
        "                  PATH | -\n"
        "\n"
        "  PATH          path to the guest byte file; '-' reads stdin.\n"
        "  --entry       guest entry PC (hex, default 0x4000).\n"
        "  --max-steps   dispatcher cap (default 1000).\n"
        "  --hex         input is whitespace-separated hex bytes.\n"
        "  --verbose     print one line per dispatched block.\n");
}

std::optional<std::uint64_t> parse_hex_u64(std::string_view s) {
    if (s.size() > 2 && s[0] == '0' && (s[1] == 'x' || s[1] == 'X')) {
        s.remove_prefix(2);
    }
    if (s.empty()) return std::nullopt;
    std::uint64_t v = 0;
    for (char c : s) {
        v <<= 4;
        if (c >= '0' && c <= '9')      v |= static_cast<std::uint64_t>(c - '0');
        else if (c >= 'a' && c <= 'f') v |= static_cast<std::uint64_t>(c - 'a' + 10);
        else if (c >= 'A' && c <= 'F') v |= static_cast<std::uint64_t>(c - 'A' + 10);
        else return std::nullopt;
    }
    return v;
}

std::vector<std::uint8_t> read_raw_file(const std::string& path) {
    std::vector<std::uint8_t> buf;
    if (path == "-") {
        char c;
        while (std::cin.get(c)) buf.push_back(static_cast<std::uint8_t>(c));
    } else {
        std::ifstream in(path, std::ios::binary);
        if (!in) {
            std::fprintf(stderr, "prisma_run: cannot open '%s'\n", path.c_str());
            std::exit(2);
        }
        std::ostringstream ss; ss << in.rdbuf();
        const std::string s = ss.str();
        buf.assign(s.begin(), s.end());
    }
    return buf;
}

std::vector<std::uint8_t> parse_hex_blob(const std::string& path) {
    std::string raw;
    if (path == "-") {
        std::ostringstream ss; ss << std::cin.rdbuf();
        raw = ss.str();
    } else {
        std::ifstream in(path);
        if (!in) {
            std::fprintf(stderr, "prisma_run: cannot open '%s'\n", path.c_str());
            std::exit(2);
        }
        std::ostringstream ss; ss << in.rdbuf();
        raw = ss.str();
    }
    std::vector<std::uint8_t> bytes;
    std::istringstream tok(raw);
    std::string word;
    while (tok >> word) {
        // Strip 0x prefix if present.
        std::string_view sv = word;
        if (sv.size() >= 2 && sv[0] == '0' && (sv[1] == 'x' || sv[1] == 'X')) {
            sv.remove_prefix(2);
        }
        if (sv.size() != 2) {
            std::fprintf(stderr,
                "prisma_run: hex input '%s' is not a single byte\n",
                std::string(sv).c_str());
            std::exit(2);
        }
        const auto v = parse_hex_u64(sv);
        if (!v.has_value() || *v > 0xFFu) {
            std::fprintf(stderr, "prisma_run: bad hex byte '%s'\n", word.c_str());
            std::exit(2);
        }
        bytes.push_back(static_cast<std::uint8_t>(*v));
    }
    return bytes;
}

const char* exit_kind_str(prisma::runtime::DispatchExit e) {
    using E = prisma::runtime::DispatchExit;
    switch (e) {
        case E::Halted:            return "Halted";
        case E::StepLimit:         return "StepLimit";
        case E::FetchFailed:       return "FetchFailed";
        case E::TranslationFailed: return "TranslationFailed";
    }
    return "Unknown";
}

}  // namespace

int main(int argc, char** argv) {
    Options opt;
    int i = 1;
    while (i < argc) {
        const std::string_view a = argv[i];
        if (a == "--entry" && i + 1 < argc) {
            const auto v = parse_hex_u64(argv[i + 1]);
            if (!v.has_value()) { print_usage(); return 2; }
            opt.entry_pc = *v;
            i += 2;
        } else if (a == "--max-steps" && i + 1 < argc) {
            opt.max_steps = static_cast<std::size_t>(std::strtoull(argv[i + 1], nullptr, 10));
            i += 2;
        } else if (a == "--hex") {
            opt.hex_input = true;
            ++i;
        } else if (a == "--verbose" || a == "-v") {
            opt.verbose = true;
            ++i;
        } else if (a == "--help" || a == "-h") {
            print_usage();
            return 0;
        } else {
            opt.path = std::string(a);
            ++i;
        }
    }
    if (opt.path.empty()) {
        print_usage();
        return 2;
    }

    auto bytes = opt.hex_input ? parse_hex_blob(opt.path) : read_raw_file(opt.path);
    if (bytes.empty()) {
        std::fprintf(stderr, "prisma_run: empty input\n");
        return 2;
    }

    if (opt.verbose) {
        std::fprintf(stderr, "prisma_run: %zu bytes, entry=0x%llx, max_steps=%zu\n",
                     bytes.size(),
                     static_cast<unsigned long long>(opt.entry_pc),
                     opt.max_steps);
    }

    prisma::translator::Translator tx;

    // Memory reader: returns the slice starting at (pc - entry_pc) into
    // the loaded blob, or empty if out of range. Translations consume
    // bytes from this point until they hit a terminator.
    const std::uint64_t base = opt.entry_pc;
    auto reader = [&](std::uint64_t pc) -> std::span<const std::uint8_t> {
        if (pc < base) return {};
        const std::size_t off = static_cast<std::size_t>(pc - base);
        if (off >= bytes.size()) return {};
        return std::span<const std::uint8_t>(bytes.data() + off,
                                             bytes.size() - off);
    };

    prisma::runtime::Dispatcher disp{tx, reader};
    auto r = disp.run(opt.entry_pc, opt.max_steps);

    // JSON-style output for easy ingestion by tools/diff-qemu / bench.
    std::printf("{\n");
    std::printf("  \"exit\":  \"%s\",\n", exit_kind_str(r.exit));
    std::printf("  \"final_pc\": \"0x%llx\",\n",
                static_cast<unsigned long long>(r.final_pc));
    std::printf("  \"blocks_executed\": %zu,\n", r.stats.blocks_executed);
    std::printf("  \"steps_taken\":     %zu,\n", r.stats.steps_taken);
    std::printf("  \"unique_pcs_seen\": %zu,\n", r.stats.unique_pcs_seen);
    std::printf("  \"rax\": \"0x%llx\",\n",
                static_cast<unsigned long long>(disp.state()[prisma::ir::Gpr::Rax]));
    std::printf("  \"rcx\": \"0x%llx\",\n",
                static_cast<unsigned long long>(disp.state()[prisma::ir::Gpr::Rcx]));
    std::printf("  \"rsp\": \"0x%llx\",\n",
                static_cast<unsigned long long>(disp.state()[prisma::ir::Gpr::Rsp]));
    std::printf("  \"message\": \"%s\"\n", r.message.c_str());
    std::printf("}\n");

    return r.exit == prisma::runtime::DispatchExit::Halted ? 0 : 1;
}
