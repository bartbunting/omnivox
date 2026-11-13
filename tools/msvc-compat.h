/* SPDX-License-Identifier: MIT
 * The pinned eSpeak headers probe include_next without a fallback definition.
 * MSVC has no include_next search; use the headers' existing Windows branches.
 * Preserve the real builtin for compilers that support it, including clang-cl.
 */
#ifndef OMNIVOX_MSVC_COMPAT_H
#define OMNIVOX_MSVC_COMPAT_H

#if defined(_MSC_VER) && !defined(__clang__)
#ifndef __has_include_next
#define __has_include_next(header) 0
#endif
#endif

#endif
