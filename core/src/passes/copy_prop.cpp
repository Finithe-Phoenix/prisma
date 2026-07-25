// core/src/passes/copy_prop.cpp — copy propagation (F1-PS-006).
//
// Finds `BinOp Or, x, x` moves (the shape CSE emits when it dedupes a
// redundant expression) and rewrites subsequent uses of the copy's
// result ref to reference the original ref directly. DCE then removes
// the now-dead copy statement.
//
// Only the `Or x, x` shape is considered. Algebraic identities that
// could generate moves (Add x, 0 etc.) are handled by algebraic_simplify
// on the emission side; a dedicated Copy IR op would also be nice but
// isn't in the IR yet.

#include "prisma/passes.hpp"

#include <unordered_map>
#include <variant>

namespace prisma::passes {

namespace {

// Resolve `r` through any chain of discovered aliases. The map is flat
// (r -> x where x has no further alias), so one lookup suffices — but
// we loop defensively in case insertion order created a chain.
ir::Ref resolve(ir::Ref r, const std::unordered_map<ir::Ref, ir::Ref>& alias) {
  while (true) {
    auto it = alias.find(r);
    if (it == alias.end())
      return r;
    if (it->second == r)
      return r;  // self-loop guard
    r = it->second;
  }
}

// Rewrite every Ref field of `op` through `alias`. Pure ops are the
// same shape as before; side-effecting ops may also carry refs.
ir::Op rewrite(ir::Op op, const std::unordered_map<ir::Ref, ir::Ref>& alias) {
  auto rewrite_optional = [&](std::optional<ir::Ref>& ref) {
    if (ref.has_value()) {
      ref = resolve(*ref, alias);
    }
  };
  return std::visit(
      [&](auto x) -> ir::Op {
        using T = std::decay_t<decltype(x)>;
        if constexpr (std::is_same_v<T, ir::StoreReg>) {
          x.value = resolve(x.value, alias);
        } else if constexpr (std::is_same_v<T, ir::BinOp>) {
          x.lhs = resolve(x.lhs, alias);
          x.rhs = resolve(x.rhs, alias);
        } else if constexpr (std::is_same_v<T, ir::WideDiv>) {
          x.high = resolve(x.high, alias);
          x.low = resolve(x.low, alias);
          x.divisor = resolve(x.divisor, alias);
        } else if constexpr (std::is_same_v<T, ir::Compare>) {
          x.lhs = resolve(x.lhs, alias);
          x.rhs = resolve(x.rhs, alias);
        } else if constexpr (std::is_same_v<T, ir::Select>) {
          x.true_value = resolve(x.true_value, alias);
          x.false_value = resolve(x.false_value, alias);
        } else if constexpr (std::is_same_v<T, ir::Extend>) {
          x.value = resolve(x.value, alias);
        } else if constexpr (std::is_same_v<T, ir::Truncate>) {
          x.value = resolve(x.value, alias);
        } else if constexpr (std::is_same_v<T, ir::LoadMem>) {
          x.addr = resolve(x.addr, alias);
        } else if constexpr (std::is_same_v<T, ir::StoreMem>) {
          x.addr = resolve(x.addr, alias);
          x.value = resolve(x.value, alias);
        } else if constexpr (std::is_same_v<T, ir::LoadMemTSO>) {
          x.addr = resolve(x.addr, alias);
        } else if constexpr (std::is_same_v<T, ir::StoreMemTSO>) {
          x.addr = resolve(x.addr, alias);
          x.value = resolve(x.value, alias);
        } else if constexpr (std::is_same_v<T, ir::AtomicCmpxchg>) {
          x.addr = resolve(x.addr, alias);
          x.expected = resolve(x.expected, alias);
          x.new_value = resolve(x.new_value, alias);
        } else if constexpr (std::is_same_v<T, ir::AtomicCmpxchgPair>) {
          x.addr = resolve(x.addr, alias);
          x.expected_low = resolve(x.expected_low, alias);
          x.expected_high = resolve(x.expected_high, alias);
          x.new_low = resolve(x.new_low, alias);
          x.new_high = resolve(x.new_high, alias);
        } else if constexpr (std::is_same_v<T, ir::StoreCarry>) {
          x.value = resolve(x.value, alias);
        } else if constexpr (std::is_same_v<T, ir::StoreRflags>) {
          x.value = resolve(x.value, alias);
        } else if constexpr (std::is_same_v<T, ir::StoreRflagsFromNzcv>) {
          rewrite_optional(x.pf);
          rewrite_optional(x.af);
        } else if constexpr (std::is_same_v<T, ir::StoreRflagsFromBits>) {
          rewrite_optional(x.pf);
          rewrite_optional(x.af);
          x.zf = resolve(x.zf, alias);
          x.sf = resolve(x.sf, alias);
          x.of = resolve(x.of, alias);
        } else if constexpr (std::is_same_v<T, ir::CmpFlags>) {
          x.lhs = resolve(x.lhs, alias);
          x.rhs = resolve(x.rhs, alias);
        } else if constexpr (std::is_same_v<T, ir::AluFlags>) {
          x.lhs = resolve(x.lhs, alias);
          x.rhs = resolve(x.rhs, alias);
        } else if constexpr (std::is_same_v<T, ir::WriteFlagsCountZero>) {
          x.src = resolve(x.src, alias);
          x.result = resolve(x.result, alias);
        } else if constexpr (std::is_same_v<T, ir::FpBinOp>) {
          x.lhs = resolve(x.lhs, alias);
          x.rhs = resolve(x.rhs, alias);
        } else if constexpr (std::is_same_v<T, ir::XmmFromGpr>) {
          x.value = resolve(x.value, alias);
        } else if constexpr (std::is_same_v<T, ir::GprFromXmm>) {
          x.value = resolve(x.value, alias);
        } else if constexpr (std::is_same_v<T, ir::VecBinOp>) {
          x.lhs = resolve(x.lhs, alias);
          x.rhs = resolve(x.rhs, alias);
        } else if constexpr (std::is_same_v<T, ir::VecClMul>) {
          x.lhs = resolve(x.lhs, alias);
          x.rhs = resolve(x.rhs, alias);
        } else if constexpr (std::is_same_v<T, ir::VecF16Cvt>) {
          x.src = resolve(x.src, alias);
        } else if constexpr (std::is_same_v<T, ir::PcmpStrIndex>) {
          x.lhs = resolve(x.lhs, alias);
          x.rhs = resolve(x.rhs, alias);
          rewrite_optional(x.lhs_len);
          rewrite_optional(x.rhs_len);
        } else if constexpr (std::is_same_v<T, ir::PcmpStrMask>) {
          x.lhs = resolve(x.lhs, alias);
          x.rhs = resolve(x.rhs, alias);
          rewrite_optional(x.lhs_len);
          rewrite_optional(x.rhs_len);
        } else if constexpr (std::is_same_v<T, ir::PcmpStrFlags>) {
          x.lhs = resolve(x.lhs, alias);
          x.rhs = resolve(x.rhs, alias);
          rewrite_optional(x.lhs_len);
          rewrite_optional(x.rhs_len);
        } else if constexpr (std::is_same_v<T, ir::CondJump>) {
          x.cond = resolve(x.cond, alias);
        } else if constexpr (std::is_same_v<T, ir::JumpReg>) {
          x.target = resolve(x.target, alias);
        } else if constexpr (std::is_same_v<T, ir::CallReg>) {
          x.target = resolve(x.target, alias);
        } else if constexpr (std::is_same_v<T, ir::TrapIf>) {
          x.condition = resolve(x.condition, alias);
        } else if constexpr (std::is_same_v<T, ir::X87Store>) {
          x.value = resolve(x.value, alias);
        } else if constexpr (std::is_same_v<T, ir::X87Push>) {
          x.value = resolve(x.value, alias);
        }
        return ir::Op{std::move(x)};
      },
      std::move(op));
}

}  // namespace

std::vector<ir::Stmt> copy_propagate(const std::vector<ir::Stmt>& stmts) {
  std::unordered_map<ir::Ref, ir::Ref> alias;

  std::vector<ir::Stmt> out;
  out.reserve(stmts.size());

  for (const auto& s : stmts) {
    // Rewrite operands first, so detection of the Or-self idiom
    // below looks at the canonical alias form.
    ir::Stmt new_stmt{s.result, rewrite(s.op, alias)};

    // Detect `%dst = Or %x, %x` — register it as an alias from
    // %dst to %x. The Or itself survives; DCE removes it when
    // its result stops being used.
    if (new_stmt.result && std::holds_alternative<ir::BinOp>(new_stmt.op)) {
      const auto& b = std::get<ir::BinOp>(new_stmt.op);
      if (b.op == ir::BinOpKind::Or && b.lhs == b.rhs) {
        alias[*new_stmt.result] = b.lhs;
      }
    }

    out.push_back(std::move(new_stmt));
  }

  return out;
}

}  // namespace prisma::passes
