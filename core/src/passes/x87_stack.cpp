// core/src/passes/x87_stack.cpp -- F2-PS-001.
//
// Conservative x87 stack forwarding. The x87 bridge intentionally keeps
// architectural state materialised in CpuStateFrame, but within a single
// decoded block we can avoid re-loading ST(i) when the slot's value is
// already known as an SSA ref.

#include "prisma/passes.hpp"

#include <array>
#include <optional>
#include <unordered_map>
#include <variant>

namespace prisma::passes {

namespace {

using KnownStack = std::array<std::optional<ir::Ref>, 8>;

ir::Ref resolve(ir::Ref r, const std::unordered_map<ir::Ref, ir::Ref>& alias) {
  while (true) {
    const auto it = alias.find(r);
    if (it == alias.end())
      return r;
    if (it->second == r)
      return r;
    r = it->second;
  }
}

ir::Op rewrite_x87_value_operands(ir::Op op, const std::unordered_map<ir::Ref, ir::Ref>& alias) {
  return std::visit(
      [&](auto x) -> ir::Op {
        using T = std::decay_t<decltype(x)>;
        if constexpr (std::is_same_v<T, ir::X87Store>) {
          x.value = resolve(x.value, alias);
        } else if constexpr (std::is_same_v<T, ir::X87Push>) {
          x.value = resolve(x.value, alias);
        }
        return ir::Op{std::move(x)};
      },
      std::move(op));
}

void push_known(KnownStack& stack, ir::Ref value) {
  for (std::size_t i = stack.size() - 1u; i > 0u; --i) {
    stack[i] = stack[i - 1u];
  }
  stack[0] = value;
}

void pop_known(KnownStack& stack) {
  for (std::size_t i = 0u; i + 1u < stack.size(); ++i) {
    stack[i] = stack[i + 1u];
  }
  stack.back().reset();
}

bool clears_x87_knowledge(const ir::Op& op) noexcept {
  return std::visit(
      [](const auto& x) -> bool {
        using T = std::decay_t<decltype(x)>;
        if constexpr (std::is_same_v<T, ir::CallRel>)
          return true;
        else if constexpr (std::is_same_v<T, ir::CallReg>)
          return true;
        else if constexpr (std::is_same_v<T, ir::RetAdjusted>)
          return true;
        else if constexpr (std::is_same_v<T, ir::Jump>)
          return true;
        else if constexpr (std::is_same_v<T, ir::JumpRel>)
          return true;
        else if constexpr (std::is_same_v<T, ir::JumpReg>)
          return true;
        else if constexpr (std::is_same_v<T, ir::CondJump>)
          return true;
        else if constexpr (std::is_same_v<T, ir::CondJumpRel>)
          return true;
        else if constexpr (std::is_same_v<T, ir::CondJumpFlags>)
          return true;
        else if constexpr (std::is_same_v<T, ir::Return>)
          return true;
        else if constexpr (std::is_same_v<T, ir::Syscall>)
          return true;
        else if constexpr (std::is_same_v<T, ir::Trap>)
          return true;
        else if constexpr (std::is_same_v<T, ir::InlineAsm>)
          return true;
        else {
          (void)x;
          return false;
        }
      },
      op);
}

}  // namespace

std::vector<ir::Stmt> x87_stack_eliminate(const std::vector<ir::Stmt>& stmts) {
  KnownStack known{};
  std::unordered_map<ir::Ref, ir::Ref> alias;

  std::vector<ir::Stmt> out;
  out.reserve(stmts.size());

  for (const auto& s : stmts) {
    ir::Op op = rewrite_x87_value_operands(s.op, alias);

    if (std::holds_alternative<ir::X87Load>(op)) {
      const auto load = std::get<ir::X87Load>(op);
      if (load.st_index < known.size() && s.result.has_value() &&
          known[load.st_index].has_value()) {
        const ir::Ref src = resolve(*known[load.st_index], alias);
        alias[*s.result] = src;
        out.push_back(ir::Stmt{
            s.result,
            ir::BinOp{ir::BinOpKind::Or, src, src, ir::OpSize::I64},
        });
      } else {
        out.push_back(ir::Stmt{s.result, std::move(op)});
        if (load.st_index < known.size() && s.result.has_value()) {
          known[load.st_index] = *s.result;
        } else {
          known = KnownStack{};
        }
      }
      continue;
    }

    if (std::holds_alternative<ir::X87Store>(op)) {
      const auto store = std::get<ir::X87Store>(op);
      if (store.st_index < known.size()) {
        known[store.st_index] = resolve(store.value, alias);
      } else {
        known = KnownStack{};
      }
      out.push_back(ir::Stmt{s.result, std::move(op)});
      continue;
    }

    if (std::holds_alternative<ir::X87Push>(op)) {
      const auto push = std::get<ir::X87Push>(op);
      push_known(known, resolve(push.value, alias));
      out.push_back(ir::Stmt{s.result, std::move(op)});
      continue;
    }

    if (std::holds_alternative<ir::X87Pop>(op)) {
      if (s.result.has_value() && known[0].has_value()) {
        alias[*s.result] = resolve(*known[0], alias);
      }
      pop_known(known);
      out.push_back(ir::Stmt{s.result, std::move(op)});
      continue;
    }

    if (clears_x87_knowledge(op)) {
      known = KnownStack{};
    }
    out.push_back(ir::Stmt{s.result, std::move(op)});
  }

  return out;
}

}  // namespace prisma::passes
