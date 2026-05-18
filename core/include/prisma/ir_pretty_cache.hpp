// prisma/ir_pretty_cache.hpp - memoized pretty-print helpers for tests/tools.

#pragma once

#include <cstddef>
#include <string>
#include <unordered_map>

#include "prisma/ir.hpp"

namespace prisma::ir {

class PrettyPrintCache {
public:
    [[nodiscard]] const std::string& render(const Op& op);
    [[nodiscard]] const std::string& render(const Stmt& stmt);
    [[nodiscard]] const std::string& render(const BasicBlock& block);
    [[nodiscard]] const std::string& render(const Function& function);

    [[nodiscard]] const std::string& pretty_print(const Op& op);
    [[nodiscard]] const std::string& pretty_print(const Stmt& stmt);
    [[nodiscard]] const std::string& pretty_print(const BasicBlock& block);
    [[nodiscard]] const std::string& pretty_print(const Function& function);

    [[nodiscard]] std::size_t size() const noexcept;
    void clear();

private:
    [[nodiscard]] const std::string& intern(std::string key, std::string text);

    std::unordered_map<std::string, std::string> cache_;
};

}  // namespace prisma::ir
