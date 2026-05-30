// core/src/ir/dominators.cpp — F1-IR-024 + F1-IR-025.

#include "prisma/dominators.hpp"

#include <algorithm>
#include <unordered_map>
#include <variant>

namespace prisma::ir {

namespace {

// Map from block id to the index in `fn.blocks`. We don't assume
// blocks are stored in id order — `build_cfg` happens to produce
// them that way today, but a future pass could reorder.
std::unordered_map<std::uint32_t, std::size_t> id_to_index(const Function& fn) {
    std::unordered_map<std::uint32_t, std::size_t> map;
    map.reserve(fn.blocks.size());
    for (std::size_t i = 0; i < fn.blocks.size(); ++i) {
        map[fn.blocks[i].id] = i;
    }
    return map;
}

// Predecessors map: pred[b] = list of blocks that can flow to b via
// successors(). O(N + E).
std::unordered_map<std::uint32_t, std::vector<std::uint32_t>>
predecessors_map(const Function& fn) {
    std::unordered_map<std::uint32_t, std::vector<std::uint32_t>> preds;
    for (const auto& b : fn.blocks) preds[b.id];  // ensure entry exists
    for (const auto& b : fn.blocks) {
        for (auto succ : successors(fn, b.id)) {
            preds[succ].push_back(b.id);
        }
    }
    return preds;
}

}  // namespace

std::vector<std::uint32_t>
successors(const Function& fn, std::uint32_t block_id) {
    auto map = id_to_index(fn);
    auto it = map.find(block_id);
    if (it == map.end()) return {};
    const auto& b = fn.blocks[it->second];
    if (b.stmts.empty()) return {};
    const auto& term = b.stmts.back().op;
    if (std::holds_alternative<Jump>(term)) {
        return {std::get<Jump>(term).target_block};
    }
    if (std::holds_alternative<CondJump>(term)) {
        const auto& cj = std::get<CondJump>(term);
        return {cj.if_true, cj.if_false};
    }
    return {};
}

std::vector<std::uint32_t> postorder(const Function& fn) {
    std::vector<std::uint32_t> order;
    if (fn.blocks.empty()) return order;
    auto idx = id_to_index(fn);
    if (idx.find(fn.entry) == idx.end()) return order;

    std::vector<bool> visited(fn.blocks.size(), false);

    // Iterative DFS — guard against pathological deep CFGs blowing the
    // stack on a recursive walk. Each entry is (block_id, child cursor).
    struct Frame { std::uint32_t id; std::size_t cursor; };
    std::vector<Frame> stack;
    stack.push_back({fn.entry, 0u});
    visited[idx[fn.entry]] = true;

    while (!stack.empty()) {
        Frame& f = stack.back();
        const auto succs = successors(fn, f.id);
        if (f.cursor < succs.size()) {
            const std::uint32_t s = succs[f.cursor++];
            const auto sit = idx.find(s);
            if (sit != idx.end() && !visited[sit->second]) {
                visited[sit->second] = true;
                stack.push_back({s, 0u});
            }
        } else {
            order.push_back(f.id);
            stack.pop_back();
        }
    }
    return order;
}

std::vector<std::uint32_t> dominators(const Function& fn) {
    std::vector<std::uint32_t> idom(fn.blocks.size(), kNoDominator);
    if (fn.blocks.empty()) return idom;

    auto idx = id_to_index(fn);
    if (idx.find(fn.entry) == idx.end()) return idom;

    // Compute reverse postorder once. For unreachable blocks they
    // simply never appear and idom[them] stays kNoDominator.
    auto po = postorder(fn);
    std::vector<std::uint32_t> rpo(po.rbegin(), po.rend());

    // Rank in RPO for each reachable block — used by `intersect`.
    std::unordered_map<std::uint32_t, std::size_t> rpo_rank;
    for (std::size_t i = 0; i < rpo.size(); ++i) rpo_rank[rpo[i]] = i;

    auto preds = predecessors_map(fn);

    // intersect(b1, b2): walk both up the dominator tree until they
    // meet. Cooper-Harvey-Kennedy.
    auto intersect = [&](std::uint32_t b1, std::uint32_t b2) {
        while (b1 != b2) {
            while (rpo_rank[b1] > rpo_rank[b2]) {
                b1 = idom[idx[b1]];
            }
            while (rpo_rank[b2] > rpo_rank[b1]) {
                b2 = idom[idx[b2]];
            }
        }
        return b1;
    };

    // Initialise: entry dominates itself.
    idom[idx[fn.entry]] = fn.entry;

    bool changed = true;
    while (changed) {
        changed = false;
        for (auto b : rpo) {
            if (b == fn.entry) continue;
            std::uint32_t new_idom = kNoDominator;
            for (auto p : preds[b]) {
                if (idom[idx[p]] == kNoDominator) continue;
                if (new_idom == kNoDominator) new_idom = p;
                else                          new_idom = intersect(new_idom, p);
            }
            if (new_idom != idom[idx[b]]) {
                idom[idx[b]] = new_idom;
                changed = true;
            }
        }
    }
    return idom;
}

std::vector<BackEdge> back_edges(const Function& fn) {
    std::vector<BackEdge> result;
    auto idx = id_to_index(fn);
    auto idoms = dominators(fn);

    // Helper: does `head` dominate `tail`? Walk up tail's dom chain.
    auto dominates = [&](std::uint32_t head, std::uint32_t tail) {
        auto it = idx.find(tail);
        if (it == idx.end()) return false;
        std::uint32_t cur = tail;
        for (int safety = 0; safety < 1'000'000; ++safety) {
            if (cur == head) return true;
            const std::uint32_t parent = idoms[idx[cur]];
            if (parent == kNoDominator) return false;
            if (parent == cur) return cur == head;  // entry self-dom
            cur = parent;
        }
        return false;
    };

    for (const auto& b : fn.blocks) {
        for (auto s : successors(fn, b.id)) {
            if (dominates(s, b.id)) {
                result.push_back(BackEdge{b.id, s});
            }
        }
    }
    return result;
}

std::vector<NaturalLoop> natural_loops(const Function& fn) {
    std::vector<NaturalLoop> loops;
    auto preds = predecessors_map(fn);
    auto idx = id_to_index(fn);

    for (const auto& edge : back_edges(fn)) {
        NaturalLoop loop{edge.header, edge.tail, {edge.header}};

        // Reverse BFS from tail, stopping at header.
        std::vector<std::uint32_t> worklist;
        std::vector<bool> in_loop(fn.blocks.size(), false);
        if (auto it = idx.find(edge.header); it != idx.end()) {
            in_loop[it->second] = true;
        }
        if (edge.tail != edge.header) {
            worklist.push_back(edge.tail);
            if (auto it = idx.find(edge.tail); it != idx.end()) {
                in_loop[it->second] = true;
                loop.body.push_back(edge.tail);
            }
        }
        while (!worklist.empty()) {
            const std::uint32_t b = worklist.back();
            worklist.pop_back();
            for (auto p : preds[b]) {
                auto pit = idx.find(p);
                if (pit == idx.end() || in_loop[pit->second]) continue;
                in_loop[pit->second] = true;
                loop.body.push_back(p);
                worklist.push_back(p);
            }
        }
        std::sort(loop.body.begin(), loop.body.end());
        loops.push_back(std::move(loop));
    }
    return loops;
}

}  // namespace prisma::ir
