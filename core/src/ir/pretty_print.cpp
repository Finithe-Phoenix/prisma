// prisma/ir/pretty_print.cpp — debug textual form of IR.
//
// Not performance critical. Format is not stable — used for test assertions
// and debugger output only. Do not parse it.

#include "prisma/ir.hpp"

#include <sstream>
#include <string_view>

namespace prisma::ir {

namespace {

constexpr std::string_view gpr_name(Gpr r) noexcept {
    switch (r) {
        case Gpr::Rax: return "rax"; case Gpr::Rcx: return "rcx";
        case Gpr::Rdx: return "rdx"; case Gpr::Rbx: return "rbx";
        case Gpr::Rsp: return "rsp"; case Gpr::Rbp: return "rbp";
        case Gpr::Rsi: return "rsi"; case Gpr::Rdi: return "rdi";
        case Gpr::R8:  return "r8";  case Gpr::R9:  return "r9";
        case Gpr::R10: return "r10"; case Gpr::R11: return "r11";
        case Gpr::R12: return "r12"; case Gpr::R13: return "r13";
        case Gpr::R14: return "r14"; case Gpr::R15: return "r15";
    }
    return "?";
}

constexpr std::string_view size_suffix(OpSize s) noexcept {
    switch (s) {
        case OpSize::I8:  return "i8";
        case OpSize::I16: return "i16";
        case OpSize::I32: return "i32";
        case OpSize::I64: return "i64";
    }
    return "?";
}

constexpr std::string_view binop_name(BinOpKind k) noexcept {
    switch (k) {
        case BinOpKind::Add: return "add"; case BinOpKind::Sub: return "sub";
        case BinOpKind::Mul: return "mul";
        case BinOpKind::And: return "and"; case BinOpKind::Or:  return "or";
        case BinOpKind::Xor: return "xor"; case BinOpKind::Shl: return "shl";
        case BinOpKind::Shr: return "shr"; case BinOpKind::Sar: return "sar";
        case BinOpKind::Rol: return "rol"; case BinOpKind::Ror: return "ror";
        case BinOpKind::Rcl: return "rcl"; case BinOpKind::Rcr: return "rcr";
    }
    return "?";
}

constexpr std::string_view cc_name(CondCode cc) noexcept {
    switch (cc) {
        case CondCode::Eq:  return "eq";  case CondCode::Ne:  return "ne";
        case CondCode::Ult: return "ult"; case CondCode::Ule: return "ule";
        case CondCode::Ugt: return "ugt"; case CondCode::Uge: return "uge";
        case CondCode::Slt: return "slt"; case CondCode::Sle: return "sle";
        case CondCode::Sgt: return "sgt"; case CondCode::Sge: return "sge";
        case CondCode::Cc:  return "cc";  case CondCode::Nc:  return "nc";
        case CondCode::Ov:  return "ov";  case CondCode::NoOv: return "noov";
        case CondCode::Mi:  return "mi";  case CondCode::Pl:  return "pl";
    }
    return "?";
}

void print_ref(std::ostream& os, Ref r) {
    if (r == kInvalidRef) { os << "%?"; } else { os << "%" << r; }
}

}  // namespace

std::string pretty_print(const Op& op) {
    std::ostringstream os;
    std::visit([&](auto const& x) {
        using T = std::decay_t<decltype(x)>;
        if constexpr (std::is_same_v<T, Constant>) {
            os << "const." << size_suffix(x.size) << " 0x" << std::hex << x.value;
        } else if constexpr (std::is_same_v<T, LoadReg>) {
            os << "loadreg." << size_suffix(x.size) << " " << gpr_name(x.reg);
        } else if constexpr (std::is_same_v<T, StoreReg>) {
            os << "storereg." << size_suffix(x.size) << " " << gpr_name(x.reg) << ", ";
            print_ref(os, x.value);
        } else if constexpr (std::is_same_v<T, LoadSegBase>) {
            const char* s = "?";
            switch (x.seg) {
                case SegmentReg::Es: s = "es"; break;
                case SegmentReg::Cs: s = "cs"; break;
                case SegmentReg::Ss: s = "ss"; break;
                case SegmentReg::Ds: s = "ds"; break;
                case SegmentReg::Fs: s = "fs"; break;
                case SegmentReg::Gs: s = "gs"; break;
            }
            os << "segbase " << s;
        } else if constexpr (std::is_same_v<T, BinOp>) {
            os << binop_name(x.op) << "." << size_suffix(x.size) << " ";
            print_ref(os, x.lhs); os << ", "; print_ref(os, x.rhs);
        } else if constexpr (std::is_same_v<T, Compare>) {
            os << "cmp." << cc_name(x.cc) << "." << size_suffix(x.size) << " ";
            print_ref(os, x.lhs); os << ", "; print_ref(os, x.rhs);
        } else if constexpr (std::is_same_v<T, Select>) {
            os << "select." << cc_name(x.cc) << "." << size_suffix(x.size) << " ";
            print_ref(os, x.true_value); os << ", ";
            print_ref(os, x.false_value);
        } else if constexpr (std::is_same_v<T, LoadMem>) {
            os << "load." << size_suffix(x.size) << " ["; print_ref(os, x.addr); os << "]";
        } else if constexpr (std::is_same_v<T, StoreMem>) {
            os << "store." << size_suffix(x.size) << " ["; print_ref(os, x.addr);
            os << "], "; print_ref(os, x.value);
        } else if constexpr (std::is_same_v<T, LoadMemTSO>) {
            os << "load.tso." << size_suffix(x.size) << " ["; print_ref(os, x.addr); os << "]";
        } else if constexpr (std::is_same_v<T, StoreMemTSO>) {
            os << "store.tso." << size_suffix(x.size) << " ["; print_ref(os, x.addr);
            os << "], "; print_ref(os, x.value);
        } else if constexpr (std::is_same_v<T, Jump>) {
            os << "jmp bb" << x.target_block;
        } else if constexpr (std::is_same_v<T, JumpReg>) {
            os << "jmpreg ";
            print_ref(os, x.target);
        } else if constexpr (std::is_same_v<T, CondJump>) {
            os << "condjmp "; print_ref(os, x.cond);
            os << ", bb" << x.if_true << ", bb" << x.if_false;
        } else if constexpr (std::is_same_v<T, Return>) {
            os << "ret";
        } else if constexpr (std::is_same_v<T, CmpFlags>) {
            os << "cmpflags." << size_suffix(x.size) << " ";
            print_ref(os, x.lhs); os << ", "; print_ref(os, x.rhs);
        } else if constexpr (std::is_same_v<T, JumpRel>) {
            os << "jmprel 0x" << std::hex << x.target_guest_pc;
        } else if constexpr (std::is_same_v<T, CondJumpRel>) {
            os << "condjmprel." << cc_name(x.cc)
               << " taken=0x" << std::hex << x.target_guest_pc
               << ", fallthrough=0x" << std::hex << x.fallthrough_guest_pc;
        } else if constexpr (std::is_same_v<T, CallRel>) {
            os << "callrel target=0x" << std::hex << x.target_guest_pc
               << ", ret=0x" << std::hex << x.return_guest_pc;
        } else if constexpr (std::is_same_v<T, CallReg>) {
            os << "callreg "; print_ref(os, x.target);
            os << ", ret=0x" << std::hex << x.return_guest_pc;
        } else if constexpr (std::is_same_v<T, RetAdjusted>) {
            os << "ret pop=" << std::dec << x.pop_bytes;
        } else if constexpr (std::is_same_v<T, Cpuid>) {
            os << "cpuid";
        } else if constexpr (std::is_same_v<T, Syscall>) {
            os << "syscall";
        } else if constexpr (std::is_same_v<T, Trap>) {
            const char* k = "?";
            switch (x.kind) {
                case TrapKind::Sigtrap: k = "sigtrap"; break;
                case TrapKind::Sigill:  k = "sigill";  break;
                case TrapKind::Sigfpe:  k = "sigfpe";  break;
            }
            os << "trap " << k;
        } else if constexpr (std::is_same_v<T, Extend>) {
            os << (x.is_signed ? "sext." : "zext.")
               << size_suffix(x.from_size) << "->" << size_suffix(x.to_size) << " ";
            print_ref(os, x.value);
        } else if constexpr (std::is_same_v<T, Truncate>) {
            os << "trunc->" << size_suffix(x.to_size) << " ";
            print_ref(os, x.value);
        } else if constexpr (std::is_same_v<T, Fence>) {
            const char* k = "?";
            switch (x.kind) {
                case FenceKind::Mfence: k = "mfence"; break;
                case FenceKind::Lfence: k = "lfence"; break;
                case FenceKind::Sfence: k = "sfence"; break;
            }
            os << "fence." << k;
        } else if constexpr (std::is_same_v<T, GuestPc>) {
            os << "guest_pc 0x" << std::hex << x.pc;
        } else if constexpr (std::is_same_v<T, InlineAsm>) {
            os << "inline_asm " << std::dec << x.bytes.size() << "B";
        } else if constexpr (std::is_same_v<T, FpConstant>) {
            const char* sz = x.size == FpSize::F32 ? "f32" : "f64";
            os << "fpconst." << sz << " 0x" << std::hex << x.bits;
        } else if constexpr (std::is_same_v<T, FpBinOp>) {
            const char* op_n = "?";
            switch (x.op) {
                case FpBinOpKind::Add: op_n = "fadd"; break;
                case FpBinOpKind::Sub: op_n = "fsub"; break;
                case FpBinOpKind::Mul: op_n = "fmul"; break;
                case FpBinOpKind::Div: op_n = "fdiv"; break;
            }
            const char* sz = x.size == FpSize::F32 ? "f32" : "f64";
            os << op_n << "." << sz << " ";
            print_ref(os, x.lhs); os << ", "; print_ref(os, x.rhs);
        } else if constexpr (std::is_same_v<T, WriteFlags>) {
            os << "writeflags." << binop_name(x.op) << "."
               << size_suffix(x.size) << " ";
            print_ref(os, x.lhs); os << ", "; print_ref(os, x.rhs);
        } else if constexpr (std::is_same_v<T, ReadFlag>) {
            const char* w = "?";
            switch (x.which) {
                case FlagBit::Carry:    w = "cf"; break;
                case FlagBit::Zero:     w = "zf"; break;
                case FlagBit::Sign:     w = "sf"; break;
                case FlagBit::Overflow: w = "of"; break;
                case FlagBit::Parity:   w = "pf"; break;
                case FlagBit::Aux:      w = "af"; break;
            }
            os << "readflag." << w << " "; print_ref(os, x.flags);
        } else if constexpr (std::is_same_v<T, CondJumpFlags>) {
            os << "condjmpflags." << cc_name(x.cc) << " ";
            print_ref(os, x.flags);
            os << ", bb" << std::dec << x.if_true
               << ", bb" << std::dec << x.if_false;
        } else if constexpr (std::is_same_v<T, RspAdjust>) {
            os << "rsp_adjust " << std::dec << x.delta_bytes;
        } else if constexpr (std::is_same_v<T, VecConstant>) {
            os << "vconst.128 0x" << std::hex << x.hi << ":0x" << std::hex << x.lo;
        } else if constexpr (std::is_same_v<T, LoadVecReg>) {
            os << "loadxmm xmm" << std::dec
               << static_cast<unsigned>(x.xmm_index);
        } else if constexpr (std::is_same_v<T, StoreVecReg>) {
            os << "storexmm xmm" << std::dec
               << static_cast<unsigned>(x.xmm_index) << ", ";
            print_ref(os, x.value);
        } else if constexpr (std::is_same_v<T, VecBinOp>) {
            const char* op_n = "?";
            switch (x.op) {
                case VecBinOpKind::Add: op_n = "vadd"; break;
                case VecBinOpKind::Sub: op_n = "vsub"; break;
                case VecBinOpKind::And: op_n = "vand"; break;
                case VecBinOpKind::Or:  op_n = "vorr"; break;
                case VecBinOpKind::Xor: op_n = "veor"; break;
            }
            const char* lane_n = "?";
            switch (x.lane) {
                case VecLane::B16: lane_n = "b16"; break;
                case VecLane::H8:  lane_n = "h8";  break;
                case VecLane::S4:  lane_n = "s4";  break;
                case VecLane::D2:  lane_n = "d2";  break;
            }
            os << op_n << "." << lane_n << " ";
            print_ref(os, x.lhs); os << ", "; print_ref(os, x.rhs);
        } else if constexpr (std::is_same_v<T, VecFpBinOp>) {
            const char* op_n = "?";
            switch (x.op) {
                case VecFpBinOpKind::Add: op_n = "vfadd"; break;
                case VecFpBinOpKind::Sub: op_n = "vfsub"; break;
                case VecFpBinOpKind::Mul: op_n = "vfmul"; break;
                case VecFpBinOpKind::Div: op_n = "vfdiv"; break;
            }
            const char* size_n = (x.size == VecFpSize::S4) ? "s4" : "d2";
            os << op_n << "." << size_n << " ";
            print_ref(os, x.lhs); os << ", "; print_ref(os, x.rhs);
        }
    }, op);
    return os.str();
}

std::string pretty_print(const Stmt& stmt) {
    std::ostringstream os;
    if (stmt.result) {
        print_ref(os, *stmt.result);
        os << " = ";
    }
    os << pretty_print(stmt.op);
    return os.str();
}

std::string pretty_print(const BasicBlock& block) {
    std::ostringstream os;
    os << "bb" << block.id << ":\n";
    for (const auto& stmt : block.stmts) {
        os << "    " << pretty_print(stmt) << "\n";
    }
    return os.str();
}

std::string pretty_print(const Function& fn) {
    std::ostringstream os;
    os << "function (entry=bb" << fn.entry << ") {\n";
    for (const auto& b : fn.blocks) {
        os << pretty_print(b);
    }
    os << "}\n";
    return os.str();
}

// ---------------------------------------------------------------------------
// F1-IR-019 — memoised pretty_print for Stmt.
// ---------------------------------------------------------------------------
//
// Per-thread bounded cache. We use std::vector<pair<Stmt,string>> rather
// than std::unordered_map because Stmt isn't trivially hashable and the
// expected cache hit rate during test runs is on small constant sets;
// linear search of <=256 entries is faster than hashing through std::variant
// for that size. When the cache fills, we drop the oldest entry (FIFO).

namespace {

constexpr std::size_t kMemoisedCapacity = 256;

struct MemoEntry {
    Stmt        key;
    std::string value;
};

std::vector<MemoEntry>& memo_storage() {
    thread_local std::vector<MemoEntry> storage;
    return storage;
}

}  // namespace

std::string pretty_print_memoised(const Stmt& stmt) {
    auto& store = memo_storage();
    for (const auto& e : store) {
        if (e.key == stmt) return e.value;
    }
    std::string s = pretty_print(stmt);
    if (store.size() >= kMemoisedCapacity) {
        store.erase(store.begin());  // FIFO eviction
    }
    store.push_back({stmt, s});
    return s;
}

void pretty_print_memoised_clear() noexcept {
    memo_storage().clear();
}

std::size_t pretty_print_memoised_size() noexcept {
    return memo_storage().size();
}

}  // namespace prisma::ir
