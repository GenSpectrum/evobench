cmake_minimum_required(VERSION 3.1)
project(evobench)

# Create a library target from your single source file.
add_library(evobench src/evobench.cpp)

# Specify include directories for consumers.
target_include_directories(evobench PUBLIC ${CMAKE_CURRENT_SOURCE_DIR}/include)

# Optionally, set properties such as C++ standard.
target_compile_features(evobench PUBLIC cxx_std_17)

# XX? -O2 -Wall -Wextra
