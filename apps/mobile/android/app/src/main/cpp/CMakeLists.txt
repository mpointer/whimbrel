cmake_minimum_required(VERSION 3.22.1)
project(whimbrel_jni C CXX)

# Passed by Gradle: the core static archive and its header directory.
add_library(whimbrel_core STATIC IMPORTED)
set_target_properties(whimbrel_core PROPERTIES IMPORTED_LOCATION "${WHIMBREL_CORE_A}")

add_library(whimbrel_jni SHARED whimbrel_jni.c)
target_include_directories(whimbrel_jni PRIVATE "${WHIMBREL_CORE_INCLUDE}")
# whisper.cpp inside the archive is C++: link as C++ so the driver adds libc++_static plus
# libc++abi and the unwinder (a bare -lc++_static from a C link leaves new/delete/__cxa_* undefined).
set_target_properties(whimbrel_jni PROPERTIES LINKER_LANGUAGE CXX)
# 16 KB pages for Android 15+.
target_link_libraries(whimbrel_jni whimbrel_core c++_static log m dl
    -Wl,--gc-sections -Wl,-z,max-page-size=16384)
