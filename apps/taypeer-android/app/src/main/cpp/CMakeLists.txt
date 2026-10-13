cmake_minimum_required(VERSION 3.22.1)
set(ANDROID_STL c++_static CACHE STRING "Private JNI broker owns its C++ objects")
project(taypeer_biometric_pipe LANGUAGES CXX)

# The broker calls the OS JCA provider. It implements no cryptographic primitive.
add_library(taypeer_biometric_pipe SHARED biometric_key_pipe.cpp)
target_compile_features(taypeer_biometric_pipe PRIVATE cxx_std_17)
target_compile_options(taypeer_biometric_pipe PRIVATE -Wall -Wextra -Werror -fvisibility=hidden)
target_link_options(taypeer_biometric_pipe PRIVATE
    "-Wl,--exclude-libs,ALL" "-Wl,-z,max-page-size=16384" "-Wl,-z,common-page-size=16384")
