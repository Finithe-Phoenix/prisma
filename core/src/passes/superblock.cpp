// core/src/passes/superblock.cpp
#include "prisma/dominators.hpp"
#include "prisma/passes.hpp"

#include <unordered_map>
#include <unordered_set>
#include <vector>

namespace prisma::passes {

ir::Function superblock_formation(const ir::Function& function) {
  if (function.blocks.empty())
    return function;

  // Map block_id to its BasicBlock copy
  std::unordered_map<std::uint32_t, ir::BasicBlock> blocks;
  for (const auto& b : function.blocks) {
    blocks[b.id] = b;
  }

  // Build successors and predecessors maps manually (like dominators.cpp)
  std::unordered_map<std::uint32_t, std::vector<std::uint32_t>> succs;
  std::unordered_map<std::uint32_t, std::vector<std::uint32_t>> preds;

  for (const auto& b : function.blocks) {
    succs[b.id] = ir::successors(function, b.id);
    for (auto s : succs[b.id]) {
      preds[s].push_back(b.id);
    }
  }

  std::unordered_set<std::uint32_t>
      merged;  // blocks that have been merged into others and should be removed.

  // Find merge candidates iteratively
  bool changed = true;
  while (changed) {
    changed = false;
    for (const auto& block_entry : blocks) {
      auto block_id = block_entry.first;
      if (merged.find(block_id) != merged.end())
        continue;

      auto succ_it = succs.find(block_id);
      if (succ_it == succs.end() || succ_it->second.size() != 1)
        continue;

      std::uint32_t target_id = succ_it->second.front();
      if (target_id == block_id)
        continue;
      if (merged.find(target_id) != merged.end())
        continue;
      if (target_id == function.entry)
        continue;

      auto pred_it = preds.find(target_id);
      if (pred_it == preds.end() || pred_it->second.size() != 1)
        continue;

      // Target has exactly 1 predecessor (block_id), and block_id has exactly 1 successor
      // (target_id). Merge target_id into block_id
      auto& a = blocks[block_id];
      auto& b = blocks[target_id];

      if (!a.stmts.empty() && std::holds_alternative<ir::Jump>(a.stmts.back().op)) {
        a.stmts.pop_back();
      }

      a.stmts.insert(a.stmts.end(), b.stmts.begin(), b.stmts.end());
      merged.insert(target_id);

      // Update CFG state to allow cascading merges
      succs[block_id] = succs[target_id];
      for (auto succ : succs[target_id]) {
        auto& p_list = preds[succ];
        for (auto& p : p_list) {
          if (p == target_id) {
            p = block_id;
          }
        }
      }
      changed = true;
    }
  }

  if (merged.empty()) {
    return function;  // No changes
  }

  // Rebuild the function
  ir::Function result;
  result.entry = function.entry;
  for (const auto& b : function.blocks) {
    if (merged.find(b.id) == merged.end()) {
      result.blocks.push_back(std::move(blocks[b.id]));
    }
  }

  return result;
}

}  // namespace prisma::passes
