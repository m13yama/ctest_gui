#include "sample/math.hpp"

#include <stdexcept>

namespace sample {

int add(int a, int b) { return a + b; }

int subtract(int a, int b) { return a - b; }

double divide(double dividend, double divisor) {
  if (divisor == 0.0) {
    throw std::invalid_argument("division by zero");
  }
  return dividend / divisor;
}

std::uint64_t factorial(unsigned int n) {
  std::uint64_t result = 1;
  for (unsigned int i = 2; i <= n; ++i) {
    result *= i;
  }
  return result;
}

}  // namespace sample
