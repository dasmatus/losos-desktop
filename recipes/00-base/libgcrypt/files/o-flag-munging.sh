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
# This inserts rather than rewriting, because there is nothing to rewrite, and
# rather than appending, because libtool is in the way. The rule is
#
#     `echo $(LTCOMPILE) -c $(srcdir)/rndjent.c | $(o_flag_munging) `
#
# so a trailing -O0 lands after the source file. clang would accept that --
# last -O on the line wins, and `clang @rsp -c -O0` was measured to beat the
# -O2 inside the response file -- but libtool's compile-mode parser takes the
# last argument as the thing to derive a library object name from, and dies
# with `cannot determine name of library object from '-O0'`. Putting -O0
# immediately before -c keeps it after CFLAGS's @response-file (so clang still
# sees it last among optimisation flags) and leaves the source file at the
# end, which is what libtool needs.
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
#
# `o_flag_munging=` is a command-line assignment, so make hands the same
# override to *every* sub-make, not just random/'s (GNU make command-line
# variables beat every makefile assignment, conditional or not). cipher/
# Makefile.am spells the identical variable name for an unrelated GCC-bug
# workaround on tiger.o and tiger.lo, and a rewrite that touched every line
# would fire there first -- cipher/ is earlier in SUBDIRS than random/ --
# which is the error that stopped the base layer:
#
#     libtool: compile: cannot determine name of library object from '-O0'
#     make[2]: *** [Makefile:1554: tiger.lo] Error 1
#
# Restricting the rewrite to lines naming rndjent.c leaves tiger's own line
# untouched, which is what it gets from upstream's `cat` fallback already:
# clang is not the GCC version the -O2-to-O1 half of this rewrite exists for,
# so passing tiger's line through unchanged loses nothing the workaround was
# buying it.
exec sed -e '/rndjent/ s/[[:blank:]]-c[[:blank:]]/ -O0 -c /'
