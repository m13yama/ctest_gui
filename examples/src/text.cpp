#include "sample/text.hpp"

#include <algorithm>
#include <cctype>
#include <sstream>

namespace sample {

std::string to_upper(const std::string& s) {
  std::string out = s;
  std::transform(out.begin(), out.end(), out.begin(),
                 [](unsigned char c) { return static_cast<char>(std::toupper(c)); });
  return out;
}

std::vector<std::string> split(const std::string& s, char delimiter) {
  std::vector<std::string> parts;
  std::stringstream stream(s);
  std::string part;
  while (std::getline(stream, part, delimiter)) {
    parts.push_back(part);
  }
  return parts;
}

std::string trim(const std::string& s) {
  const auto is_space = [](unsigned char c) { return std::isspace(c) != 0; };
  const auto begin = std::find_if_not(s.begin(), s.end(), is_space);
  const auto end = std::find_if_not(s.rbegin(), s.rend(), is_space).base();
  return begin < end ? std::string(begin, end) : std::string();
}

}  // namespace sample
