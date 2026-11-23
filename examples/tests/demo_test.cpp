// Tests that demonstrate how each kind of ctest result looks in ctgui.
// Each case is registered as a separate ctest test in CMakeLists.txt.

#include <gtest/gtest.h>

#include <chrono>
#include <csignal>
#include <cstdlib>
#include <iostream>
#include <thread>

namespace {

void sleep_with_progress(int seconds) {
  for (int i = 1; i <= seconds; ++i) {
    std::cout << "progress " << i << "/" << seconds << std::endl;
    std::this_thread::sleep_for(std::chrono::seconds(1));
  }
}

}  // namespace

// Fails with an assertion message.
TEST(Demo, AssertionFailure) { EXPECT_EQ(41, 42) << "the answer is wrong"; }

// Crashes with a segmentation fault.
TEST(Demo, Segfault) { std::raise(SIGSEGV); }

// Aborts the process.
TEST(Demo, Abort) { std::abort(); }

// Exceeds the TIMEOUT property set in CMakeLists.txt.
TEST(Demo, Timeout) { sleep_with_progress(30); }

// Skipped with GTEST_SKIP(), reported as skipped by SKIP_REGULAR_EXPRESSION.
TEST(Demo, Skipped) { GTEST_SKIP() << "required hardware not found"; }

// Fails, but WILL_FAIL makes ctest treat it as passed.
TEST(Demo, ExpectedFailure) { FAIL() << "this failure is expected"; }

// Passes, but FAIL_REGULAR_EXPRESSION matches its output.
TEST(Demo, ErrorInOutput) { std::cout << "ERROR: invalid configuration" << std::endl; }

// Writes to both stdout and stderr.
TEST(Demo, StdoutAndStderr) {
  std::cout << "message on stdout" << std::endl;
  std::cerr << "message on stderr" << std::endl;
}

// Prints many lines to try scrolling the output view.
TEST(Demo, LongOutput) {
  for (int i = 1; i <= 200; ++i) {
    std::cout << "log line " << i << std::endl;
  }
}

// Runs for 10 seconds to try live output and the Stop button.
TEST(Demo, Slow) { sleep_with_progress(10); }

// Has the DISABLED property, so ctest never runs it.
TEST(Demo, Disabled) {}
