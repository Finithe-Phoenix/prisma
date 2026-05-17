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

constexpr std::string_view segment_name(SegmentReg s) noexcept {
    switch (s) {
        case SegmentReg::Fs: return "fs";
        case SegmentReg::Gs: return "gs";
    }
    return "?";
}

constexpr std::string_view trap_name(TrapKind k) noexcept {
    switch (k) {
        case TrapKind::Sigtrap: return "sigtrap";
    }
    return "?";
}

constexpr std::string_view fence_name(FenceKind k) noexcept {
    switch (k) {
        case FenceKind::Mfence: return "mfence";
        case FenceKind::Lfence: return "lfence";
        case FenceKind::Sfence: return "sfence";
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
        } else if constexpr (std::is_same_v<T, LoadSegBase>) {
            os << "loadsegbase " << segment_name(x.segment);
        } else if constexpr (std::is_same_v<T, StoreReg>) {
            os << "storereg." << size_suffix(x.size) << " " << gpr_name(x.reg) << ", ";
            print_ref(os, x.value);
        } else if constexpr (std::is_same_v<T, BinOp>) {
            os << binop_name(x.op) << "." << size_suffix(x.size) << " ";
            print_ref(os, x.lhs); os << ", "; print_ref(os, x.rhs);
        } else if constexpr (std::is_same_v<T, Extend>) {
            os << (x.is_signed ? "sext" : "zext")
               << "." << size_suffix(x.from_size)
               << "->" << size_suffix(x.to_size) << " ";
            print_ref(os, x.value);
        } else if constexpr (std::is_same_v<T, Truncate>) {
            os << "trunc." << size_suffix(x.to_size) << " ";
            print_ref(os, x.value);
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
        } else if constexpr (std::is_same_v<T, CallRel>) {
            os << "callrel target=0x" << std::hex << x.target_guest_pc
               << ", ret=0x" << std::hex << x.return_guest_pc;
        } else if constexpr (std::is_same_v<T, CallReg>) {
            os << "callreg ";
            print_ref(os, x.target);
            os << ", ret=0x" << std::hex << x.return_guest_pc;
        } else if constexpr (std::is_same_v<T, RetAdjusted>) {
            os << "retadjusted " << std::dec << x.pop_bytes;
        } else if constexpr (std::is_same_v<T, Cpuid>) {
            os << "cpuid";
        } else if constexpr (std::is_same_v<T, Syscall>) {
            os << "syscall";
        } else if constexpr (std::is_same_v<T, Trap>) {
            os << "trap." << trap_name(x.kind);
        } else if constexpr (std::is_same_v<T, Fence>) {
            os << "fence." << fence_name(x.kind);
        } else if constexpr (std::is_same_v<T, CondJumpRel>) {
            os << "condjmprel." << cc_name(x.cc)
               << " taken=0x" << std::hex << x.target_guest_pc
               << ", fallthrough=0x" << std::hex << x.fallthrough_guest_pc;
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

}  // namespace prisma::ir
