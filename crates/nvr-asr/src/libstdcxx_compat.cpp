// libstdc++ compatibility shim for toolchains older than GCC 11.
//
// The prebuilt sherpa-onnx static libs are built with GCC >= 11 and reference
// four libstdc++ symbols that GCC 9/10 do not export (GLIBCXX_3.4.29 /
// CXXABI_1.3.13). Defining them here lets the workspace link in older cross
// images (e.g. Ubuntu 20.04) and run against an older libstdc++. With GCC >= 11
// or a non-libstdc++ standard library this file compiles to nothing.
#include <cstddef>

#if defined(__GLIBCXX__) && defined(_GLIBCXX_RELEASE) && _GLIBCXX_RELEASE < 11

#include <cstring>
#include <exception>
#include <new>
#include <string>

// C++20 no-argument std::string::reserve(): a non-binding shrink request, so
// doing nothing is conforming.
void nvr_compat_string_reserve(std::string *) noexcept
    __asm__("_ZNSt7__cxx1112basic_stringIcSt11char_traitsIcESaIcEE7reserveEv");
void nvr_compat_string_reserve(std::string *) noexcept {}

[[noreturn]] void nvr_compat_throw_bad_array_new_length()
    __asm__("_ZSt28__throw_bad_array_new_lengthv");
void nvr_compat_throw_bad_array_new_length() { throw std::bad_array_new_length(); }

// exception_ptr reference counting, delegated to the copy constructor and
// destructor the older libstdc++ does export.
void nvr_compat_exception_ptr_addref(std::exception_ptr *self) noexcept
    __asm__("_ZNSt15__exception_ptr13exception_ptr9_M_addrefEv");
void nvr_compat_exception_ptr_addref(std::exception_ptr *self) noexcept {
    // The copy takes a reference; leaking it keeps that reference.
    alignas(std::exception_ptr) unsigned char copy[sizeof(std::exception_ptr)];
    new (copy) std::exception_ptr(*self);
}

void nvr_compat_exception_ptr_release(std::exception_ptr *self) noexcept
    __asm__("_ZNSt15__exception_ptr13exception_ptr10_M_releaseEv");
void nvr_compat_exception_ptr_release(std::exception_ptr *self) noexcept {
    // Destroying a bitwise alias drops one reference without touching *self.
    alignas(std::exception_ptr) unsigned char alias[sizeof(std::exception_ptr)];
    std::memcpy(alias, static_cast<void *>(self), sizeof(alias));
    reinterpret_cast<std::exception_ptr *>(alias)->~exception_ptr();
}

#endif
