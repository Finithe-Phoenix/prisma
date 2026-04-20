// core/tests/test_passes_mem.cpp — redundant_load_eliminate,
// dead_store_eliminate, and PassRunStats timing.

#include <catch2/catch_test_macros.hpp>
#include <variant>

#include "prisma/ir.hpp"
#include "prisma/passes.hpp"

using namespace prisma;

// ---------------------------------------------------------------------
// redundant_load_eliminate
// ---------------------------------------------------------------------

TEST_CASE("rle: second LoadMem with same addr+size becomes a copy") {
    // %0 = loadreg rax            (addr)
    // %1 = loadmem %0, I64        (first load)
    // %2 = loadmem %0, I64        (redundant → Or %1, %1)
    std::vector<ir::Stmt> s = {
        {0u, ir::LoadReg{ir::Gpr::Rax, ir::OpSize::I64}},
        {1u, ir::LoadMem{0u, ir::OpSize::I64}},
        {2u, ir::LoadMem{0u, ir::OpSize::I64}},
    };
    auto out = passes::redundant_load_eliminate(s);
    REQUIRE(out.size() == 3);
    REQUIRE(out[2].op == ir::Op{ir::BinOp{
        ir::BinOpKind::Or, 1u, 1u, ir::OpSize::I64}});
}

TEST_CASE("rle: different sizes are NOT deduped") {
    std::vector<ir::Stmt> s = {
        {0u, ir::LoadReg{ir::Gpr::Rax, ir::OpSize::I64}},
        {1u, ir::LoadMem{0u, ir::OpSize::I32}},
        {2u, ir::LoadMem{0u, ir::OpSize::I64}},  // different size — new load
    };
    auto out = passes::redundant_load_eliminate(s);
    REQUIRE(std::holds_alternative<ir::LoadMem>(out[2].op));
}

TEST_CASE("rle: intervening StoreMem flushes the table") {
    // load, store (any addr), load — the second load is NOT redundant.
    std::vector<ir::Stmt> s = {
        {0u, ir::LoadReg{ir::Gpr::Rax, ir::OpSize::I64}},
        {1u, ir::LoadMem{0u, ir::OpSize::I64}},
        {2u, ir::LoadReg{ir::Gpr::Rbx, ir::OpSize::I64}},
        {std::nullopt, ir::StoreMem{2u, 1u, ir::OpSize::I64}},  // flush
        {3u, ir::LoadMem{0u, ir::OpSize::I64}},                 // NOT redundant
    };
    auto out = passes::redundant_load_eliminate(s);
    REQUIRE(std::holds_alternative<ir::LoadMem>(out[4].op));
}

TEST_CASE("rle: LoadMemTSO is intentionally never deduped") {
    // Two identical TSO loads — both preserved.
    std::vector<ir::Stmt> s = {
        {0u, ir::LoadReg{ir::Gpr::Rax, ir::OpSize::I64}},
        {1u, ir::LoadMemTSO{0u, ir::OpSize::I64}},
        {2u, ir::LoadMemTSO{0u, ir::OpSize::I64}},
    };
    auto out = passes::redundant_load_eliminate(s);
    REQUIRE(std::holds_alternative<ir::LoadMemTSO>(out[2].op));
}

TEST_CASE("rle: three reads of same addr all collapse to the first") {
    std::vector<ir::Stmt> s = {
        {0u, ir::LoadReg{ir::Gpr::Rax, ir::OpSize::I64}},
        {1u, ir::LoadMem{0u, ir::OpSize::I64}},
        {2u, ir::LoadMem{0u, ir::OpSize::I64}},
        {3u, ir::LoadMem{0u, ir::OpSize::I64}},
    };
    auto out = passes::redundant_load_eliminate(s);
    REQUIRE(out[2].op == ir::Op{ir::BinOp{ir::BinOpKind::Or, 1u, 1u, ir::OpSize::I64}});
    REQUIRE(out[3].op == ir::Op{ir::BinOp{ir::BinOpKind::Or, 1u, 1u, ir::OpSize::I64}});
}

// ---------------------------------------------------------------------
// dead_store_eliminate
// ---------------------------------------------------------------------

TEST_CASE("dse: two consecutive StoreMems to same addr kill the first") {
    // storemem %a, %v1        (dead)
    // storemem %a, %v2        (kept)
    std::vector<ir::Stmt> s = {
        {0u, ir::LoadReg{ir::Gpr::Rax, ir::OpSize::I64}},  // addr
        {1u, ir::Constant{1, ir::OpSize::I64}},
        {2u, ir::Constant{2, ir::OpSize::I64}},
        {std::nullopt, ir::StoreMem{0u, 1u, ir::OpSize::I64}},  // DEAD
        {std::nullopt, ir::StoreMem{0u, 2u, ir::OpSize::I64}},  // kept
    };
    auto out = passes::dead_store_eliminate(s);
    REQUIRE(out.size() == 4);
    // The surviving StoreMem writes %2 (the second value).
    int store_count = 0;
    for (const auto& st : out) {
        if (std::holds_alternative<ir::StoreMem>(st.op)) {
            ++store_count;
            REQUIRE(std::get<ir::StoreMem>(st.op).value == 2u);
        }
    }
    REQUIRE(store_count == 1);
}

TEST_CASE("dse: intervening LoadMem protects the first store") {
    std::vector<ir::Stmt> s = {
        {0u, ir::LoadReg{ir::Gpr::Rax, ir::OpSize::I64}},
        {1u, ir::Constant{1, ir::OpSize::I64}},
        {2u, ir::Constant{2, ir::OpSize::I64}},
        {std::nullopt, ir::StoreMem{0u, 1u, ir::OpSize::I64}},
        {3u, ir::LoadMem{0u, ir::OpSize::I64}},                  // reads → keep
        {std::nullopt, ir::StoreMem{0u, 2u, ir::OpSize::I64}},
    };
    auto out = passes::dead_store_eliminate(s);
    int store_count = 0;
    for (const auto& st : out) {
        if (std::holds_alternative<ir::StoreMem>(st.op)) ++store_count;
    }
    REQUIRE(store_count == 2);  // neither store eliminated
}

TEST_CASE("dse: different addr refs are not merged") {
    std::vector<ir::Stmt> s = {
        {0u, ir::LoadReg{ir::Gpr::Rax, ir::OpSize::I64}},
        {1u, ir::LoadReg{ir::Gpr::Rbx, ir::OpSize::I64}},
        {2u, ir::Constant{1, ir::OpSize::I64}},
        {std::nullopt, ir::StoreMem{0u, 2u, ir::OpSize::I64}},
        {std::nullopt, ir::StoreMem{1u, 2u, ir::OpSize::I64}},
    };
    auto out = passes::dead_store_eliminate(s);
    int store_count = 0;
    for (const auto& st : out) {
        if (std::holds_alternative<ir::StoreMem>(st.op)) ++store_count;
    }
    REQUIRE(store_count == 2);
}

TEST_CASE("dse: StoreMemTSO flushes the pending table (conservative)") {
    // The TSO store in the middle means the first plain store is
    // observable and must not be eliminated.
    std::vector<ir::Stmt> s = {
        {0u, ir::LoadReg{ir::Gpr::Rax, ir::OpSize::I64}},
        {1u, ir::Constant{1, ir::OpSize::I64}},
        {2u, ir::Constant{2, ir::OpSize::I64}},
        {std::nullopt, ir::StoreMem{0u, 1u, ir::OpSize::I64}},
        {std::nullopt, ir::StoreMemTSO{0u, 2u, ir::OpSize::I64}},
        {std::nullopt, ir::StoreMem{0u, 2u, ir::OpSize::I64}},
    };
    auto out = passes::dead_store_eliminate(s);
    int plain = 0;
    for (const auto& st : out) {
        if (std::holds_alternative<ir::StoreMem>(st.op)) ++plain;
    }
    REQUIRE(plain == 2);  // nothing killed
}

TEST_CASE("dse: three writes in a row leave only the last") {
    std::vector<ir::Stmt> s = {
        {0u, ir::LoadReg{ir::Gpr::Rax, ir::OpSize::I64}},
        {1u, ir::Constant{1, ir::OpSize::I64}},
        {2u, ir::Constant{2, ir::OpSize::I64}},
        {3u, ir::Constant{3, ir::OpSize::I64}},
        {std::nullopt, ir::StoreMem{0u, 1u, ir::OpSize::I64}},
        {std::nullopt, ir::StoreMem{0u, 2u, ir::OpSize::I64}},
        {std::nullopt, ir::StoreMem{0u, 3u, ir::OpSize::I64}},
    };
    auto out = passes::dead_store_eliminate(s);
    int plain = 0;
    std::uint64_t surviving_value_ref = 0;
    for (const auto& st : out) {
        if (std::holds_alternative<ir::StoreMem>(st.op)) {
            ++plain;
            surviving_value_ref = std::get<ir::StoreMem>(st.op).value;
        }
    }
    REQUIRE(plain == 1);
    REQUIRE(surviving_value_ref == 3u);
}

// ---------------------------------------------------------------------
// PassRunStats timing (F1-PS-016)
// ---------------------------------------------------------------------

TEST_CASE("PassManager: stats include non-zero per-pass timing") {
    // Build a pipeline with a pass whose cost we can sanity-check.
    // We don't assert an exact duration — CI variance is brutal — we
    // only assert the field was populated.
    std::vector<ir::Stmt> s = {
        {0u, ir::Constant{1, ir::OpSize::I64}},
        {1u, ir::Constant{2, ir::OpSize::I64}},
        {2u, ir::BinOp{ir::BinOpKind::Add, 0u, 1u, ir::OpSize::I64}},
        {std::nullopt, ir::StoreReg{ir::Gpr::Rax, 2u, ir::OpSize::I64}},
    };
    auto pm = passes::default_pipeline();
    auto [out, stats] = pm.run(s);

    REQUIRE(stats.passes.size() == pm.size());
    // Every entry carries *some* measurement. On a hot loop a pass could
    // round to 0 ns, so just check that at least one pass reported > 0
    // — which is a conservative but meaningful signal that timing wired
    // through correctly.
    std::uint64_t total = 0;
    for (const auto& entry : stats.passes) total += entry.duration_ns;
    REQUIRE(total > 0);
}

TEST_CASE("PassManager: default_pipeline now has 10 passes") {
    auto pm = passes::default_pipeline();
    REQUIRE(pm.size() == 10u);
}

// ---------------------------------------------------------------------
// End-to-end: redundant-load + dead-store interact correctly with DCE
// ---------------------------------------------------------------------

TEST_CASE("pipeline: redundant load + dead store collapse to one load+store") {
    // Two loads of the same addr, then two stores to a different addr —
    // after the full pipeline the redundant load becomes a copy that
    // DCE removes, and both stores survive (distinct addresses).
    std::vector<ir::Stmt> s = {
        {0u, ir::LoadReg{ir::Gpr::Rax, ir::OpSize::I64}},    // src addr
        {1u, ir::LoadReg{ir::Gpr::Rbx, ir::OpSize::I64}},    // dst addr
        {2u, ir::LoadMem{0u, ir::OpSize::I64}},
        {3u, ir::LoadMem{0u, ir::OpSize::I64}},              // redundant
        {std::nullopt, ir::StoreMem{1u, 2u, ir::OpSize::I64}},
        {std::nullopt, ir::StoreMem{1u, 3u, ir::OpSize::I64}},  // DSE → first is dead
    };
    auto pm = passes::default_pipeline();
    auto [out, _stats] = pm.run(s);

    int load_count = 0, store_count = 0;
    for (const auto& st : out) {
        if (std::holds_alternative<ir::LoadMem>(st.op)) ++load_count;
        if (std::holds_alternative<ir::StoreMem>(st.op)) ++store_count;
    }
    REQUIRE(load_count == 1);
    REQUIRE(store_count == 1);
}
