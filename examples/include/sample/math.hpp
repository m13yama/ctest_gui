#pragma once

#include <cstdint>

namespace sample {

int add(int a, int b);
int subtract(int a, int b);
// Throws std::invalid_argument when divisor is zero.
double divide(double dividend, double divisor);
std::uint64_t factorial(unsigned int n);

}  // namespace sample
