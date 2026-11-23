#include <gtest/gtest.h>

#include <stdexcept>

#include "sample/math.hpp"

TEST(Math, Add) { EXPECT_EQ(sample::add(2, 3), 5); }

TEST(Math, Subtract) { EXPECT_EQ(sample::subtract(2, 3), -1); }

TEST(Math, Divide) { EXPECT_DOUBLE_EQ(sample::divide(9.0, 3.0), 3.0); }

TEST(Math, DivideByZero) { EXPECT_THROW(sample::divide(1.0, 0.0), std::invalid_argument); }

TEST(Math, Factorial) {
  EXPECT_EQ(sample::factorial(0), 1u);
  EXPECT_EQ(sample::factorial(5), 120u);
}
