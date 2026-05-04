// core/src/passes/peephole.cpp — F1-PS-009.

#include "prisma/passes.hpp"

#include <variant>

namespace prisma::passes {

std::vector<ir::Stmt>
peephole_optimise(const std::vector<ir::Stmt>& stmts,
                  std::span<const PeepholeRule* const> rules) {
    std::vector<ir::Stmt> work = stmts;

    for (std::size_t iter = 0; iter < kPeepholeMaxIterations; ++iter) {
        bool changed = false;
        std::vector<ir::Stmt> next;
        next.reserve(work.size());

        std::size_t i = 0;
        while (i < work.size()) {
            // Try every rule at this position. First match wins.
            std::optional<PeepholeMatch> hit;
            for (const auto* r : rules) {
                hit = r->match(work, i);
                if (hit.has_value()) break;
            }
            if (hit.has_value()) {
                for (auto& s : hit->replacement) next.push_back(std::move(s));
                i += hit->consumed;
                changed = true;
            } else {
                next.push_back(work[i]);
                ++i;
            }
        }
        work = std::move(next);
        if (!changed) break;
    }
    return work;
}

namespace {

// Rule: BinOp Xor where lhs == rhs → Constant 0 (any size). The result
// ref is preserved so downstream uses keep working.
class XorSelfRule final : public PeepholeRule {
public:
    [[nodiscard]] std::optional<PeepholeMatch>
    match(const std::vector<ir::Stmt>& stmts, std::size_t idx) const override {
        const auto& s = stmts[idx];
        if (!std::holds_alternative<ir::BinOp>(s.op)) return std::nullopt;
        const auto& b = std::get<ir::BinOp>(s.op);
        if (b.op != ir::BinOpKind::Xor) return std::nullopt;
        if (b.lhs != b.rhs)             return std::nullopt;
        if (!s.result.has_value())      return std::nullopt;
        return PeepholeMatch{
            /*consumed=*/1u,
            /*replacement=*/{ir::Stmt{s.result, ir::Constant{0u, b.size}}}};
    }
    [[nodiscard]] std::string_view name() const noexcept override {
        return "xor_self_to_zero";
    }
};

// Rule: identity Extend (from_size == to_size) → no-op rewrite. Replace
// with a single-stmt that just propagates the source ref through a
// trivial Truncate (which is itself an identity at this size). This
// keeps the SSA shape simple.  Better: drop the stmt and rewire all
// later uses to the source ref. The simpler short-term variant: lower
// the Extend to Truncate{value, to_size}, which the existing
// const_prop and the const_fold pass already collapse further.
class IdentityExtendRule final : public PeepholeRule {
public:
    [[nodiscard]] std::optional<PeepholeMatch>
    match(const std::vector<ir::Stmt>& stmts, std::size_t idx) const override {
        const auto& s = stmts[idx];
        if (!std::holds_alternative<ir::Extend>(s.op)) return std::nullopt;
        const auto& e = std::get<ir::Extend>(s.op);
        if (e.from_size != e.to_size) return std::nullopt;
        if (!s.result.has_value())    return std::nullopt;
        return PeepholeMatch{
            /*consumed=*/1u,
            /*replacement=*/{ir::Stmt{s.result, ir::Truncate{e.value, e.to_size}}}};
    }
    [[nodiscard]] std::string_view name() const noexcept override {
        return "identity_extend_to_truncate";
    }
};

// Rule: Truncate where to_size == I64 is the identity. Rewrite to a
// pseudo-noop. Choose the same trick: emit a Truncate to the same size
// but the size is irrelevant because const_prop / DCE will collapse.
//
// Better practical effect: detect a Truncate-of-Extend chain where
// (from_size of outer Truncate) >= (to_size of inner Extend) and
// (signed-mode allows it) so the chain can collapse. That's a future
// rule; for now we just demonstrate the framework.
class RedundantTruncateRule final : public PeepholeRule {
public:
    [[nodiscard]] std::optional<PeepholeMatch>
    match(const std::vector<ir::Stmt>& stmts, std::size_t idx) const override {
        const auto& s = stmts[idx];
        if (!std::holds_alternative<ir::Truncate>(s.op)) return std::nullopt;
        const auto& t = std::get<ir::Truncate>(s.op);
        if (t.to_size != ir::OpSize::I64) return std::nullopt;
        if (!s.result.has_value())        return std::nullopt;
        // Replace with a Constant-of-mask AND of the source — but we
        // can't read the source value at peephole time. Instead, emit
        // an identity Truncate that the next const_prop/DCE pass will
        // fold (or, in a forward-DCE friendly way, just drop the
        // statement — the result Ref becomes dangling, which a real
        // ref-rewrite pass can handle in a follow-up).
        //
        // For the MVP, conservatively decline rather than produce
        // dangling refs. The framework is still exercised by the two
        // rules above.
        (void)t;
        return std::nullopt;
    }
    [[nodiscard]] std::string_view name() const noexcept override {
        return "redundant_truncate_i64_noop";
    }
};

}  // namespace

std::vector<std::unique_ptr<PeepholeRule>>
peephole_default_rules() {
    std::vector<std::unique_ptr<PeepholeRule>> rules;
    rules.emplace_back(std::make_unique<XorSelfRule>());
    rules.emplace_back(std::make_unique<IdentityExtendRule>());
    rules.emplace_back(std::make_unique<RedundantTruncateRule>());
    return rules;
}

std::vector<ir::Stmt>
peephole_optimise_default(const std::vector<ir::Stmt>& stmts) {
    auto owned = peephole_default_rules();
    std::vector<const PeepholeRule*> view;
    view.reserve(owned.size());
    for (const auto& p : owned) view.push_back(p.get());
    return peephole_optimise(stmts,
        std::span<const PeepholeRule* const>(view));
}

}  // namespace prisma::passes
