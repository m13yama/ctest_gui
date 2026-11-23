#pragma once

#include <string>
#include <vector>

namespace sample {

std::string to_upper(const std::string& s);
std::vector<std::string> split(const std::string& s, char delimiter);
std::string trim(const std::string& s);

}  // namespace sample
