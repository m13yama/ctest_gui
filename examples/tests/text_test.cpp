#include <gtest/gtest.h>

#include "sample/text.hpp"

TEST(Text, ToUpper) { EXPECT_EQ(sample::to_upper("ctgui"), "CTGUI"); }

TEST(Text, Split) {
  const auto parts = sample::split("a,b,c", ',');
  ASSERT_EQ(parts.size(), 3u);
  EXPECT_EQ(parts[1], "b");
}

TEST(Text, Trim) {
  EXPECT_EQ(sample::trim("  hello \n"), "hello");
  EXPECT_TRUE(sample::trim("   ").empty());
}
