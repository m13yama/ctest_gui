// TEST() cases with different results inside a single ctest test.

#include <gtest/gtest.h>

#include "sample/math.hpp"

TEST(Mixed, Passes) { EXPECT_EQ(sample::add(1, 1), 2); }

TEST(Mixed, Fails) { EXPECT_EQ(sample::add(2, 2), 5) << "intentional failure"; }

TEST(Mixed, Skipped) { GTEST_SKIP() << "skipped on purpose"; }

TEST(Mixed, DISABLED_NotRun) { FAIL() << "never runs"; }
