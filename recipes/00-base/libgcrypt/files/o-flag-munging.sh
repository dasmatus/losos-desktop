#!/bin/sh
# Force -O0 onto libgcrypt's rndjent compile line.
#
# random/Makefile.am:57-71 compiles rndjent.c -- and the jitterentropy sources
# it includes -- by echoing its own compile line through $(o_flag_munging), a
# sed that rewrites -O2 to -O0. jitterentropy-base.c:58 then refuses to build
# if __OPTIMIZE__ survived, and it is right to: the CPU jitter collector
# measures timing variation in its own execution, so an optimiser is free to
# delete the thing being measured.
#
# That rewrite cannot work in this tree. CFLAGS is passed as a clang
# @response-file, because pm splits a command on whitespace (C1) and a
# multi-word CFLAGS would arrive as a dozen separate arguments. The
# optimisation level is therefore inside the file rather than in the command
# text, upstream's sed matches nothing, -O2 survives and the #error fires.
# The response file is the cause -- not the compiler, and not the base image:
# Debian's LLVM 19 and the Arch base's 22 fail identically.
#
# This appends rather than rewriting, because there is nothing to rewrite.
# clang takes the last -O on the line and expands @file where it is named, so
# a trailing -O0 beats the -O2 inside the response file. Both halves were
# measured rather than reasoned: a file carrying jitterentropy's own
# __OPTIMIZE__ guard fails under `clang @rsp -c` and compiles clean under
# `clang @rsp -c -O0`.
#
# A whole program rather than a sed spelled in the recipe, because make runs
# it as `| $(o_flag_munging)` and pm's step has to name it in one word.
# `o_flag_munging=/bin/sh <script>` cannot be spelled at all: pm would split
# it on the space and hand make a stray target.
#
# The alternative is --disable-jent-support, which builds and quietly drops an
# entropy source from the library the rest of this tree gets randomness
# through. That is a security decision rather than a build one, so it is not
# taken here to get a green build.
exec sed -e 's/$/ -O0/'
