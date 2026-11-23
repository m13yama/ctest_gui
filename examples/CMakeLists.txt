cmake_minimum_required(VERSION 3.16)
project(ctgui_example LANGUAGES CXX)

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)

find_package(GTest REQUIRED)
include(GoogleTest)

add_library(sample src/math.cpp src/text.cpp)
target_include_directories(sample PUBLIC include)

enable_testing()

# One ctest test that runs all TEST() cases of the executable
# (the same as ament_add_gtest() in ROS 2).
add_executable(math_test tests/math_test.cpp)
target_link_libraries(math_test PRIVATE sample GTest::gtest_main)
add_test(NAME math_test COMMAND math_test)
set_tests_properties(math_test PROPERTIES LABELS "unit;math")

# One ctest test per TEST() case.
add_executable(text_test tests/text_test.cpp)
target_link_libraries(text_test PRIVATE sample GTest::gtest_main)
gtest_discover_tests(text_test PROPERTIES LABELS "unit;text")

# Passed, failed, skipped and disabled TEST() cases inside one ctest test.
add_executable(mixed_test tests/mixed_test.cpp)
target_link_libraries(mixed_test PRIVATE sample GTest::gtest_main)
add_test(NAME mixed_test COMMAND mixed_test)
set_tests_properties(mixed_test PROPERTIES LABELS "demo")

# Demo tests that show each kind of ctest result.
# Registers Demo.<case> as the ctest test demo_test.<case> with extra properties.
add_executable(demo_test tests/demo_test.cpp)
target_link_libraries(demo_test PRIVATE GTest::gtest_main)
function(add_demo_test case)
  add_test(NAME demo_test.${case} COMMAND demo_test --gtest_filter=Demo.${case})
  set_tests_properties(demo_test.${case} PROPERTIES LABELS "demo" ${ARGN})
endfunction()
add_demo_test(AssertionFailure)
add_demo_test(Segfault)
add_demo_test(Abort)
add_demo_test(Timeout TIMEOUT 3)
add_demo_test(Skipped SKIP_REGULAR_EXPRESSION "\\[  SKIPPED \\]")
add_demo_test(ExpectedFailure WILL_FAIL TRUE)
add_demo_test(ErrorInOutput FAIL_REGULAR_EXPRESSION "ERROR")
add_demo_test(StdoutAndStderr)
add_demo_test(LongOutput)
add_demo_test(Slow)
add_demo_test(Disabled DISABLED TRUE)
