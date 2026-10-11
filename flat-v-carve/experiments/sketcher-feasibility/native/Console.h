#pragma once
#include <cstdio>
#include <cstdarg>
// Native equivalent of the wrapper's browser logger. Solver code is unchanged.
class Console { public: static void Log(const char* format, ...) {
    va_list args; va_start(args, format); vfprintf(stderr, format, args); va_end(args);
}};
