# losos-release: the OS identity, and the smallest real compile in the tree.
#
# This package exists twice over. It genuinely belongs in the OS -- something
# has to write /usr/lib/os-release and something has to read it back -- and it
# is the package that proves the build chain works end to end: it is configured
# by the meson that the layer below it built, compiled by the host toolchain
# inside the jail, and installed into both /dest and the sysroot.
project(
  'losos-release',
  'c',
  version: '0.1.0',
  default_options: ['c_std=c11', 'warning_level=3', 'werror=true'],
)

conf = configuration_data()
conf.set_quoted('LOSOS_VERSION', meson.project_version())
conf.set_quoted('LOSOS_OSRELEASE', get_option('prefix') / 'lib' / 'os-release')
configure_file(output: 'config.h', configuration: conf)

executable(
  'losos-release',
  'src/losos-release.c',
  install: true,
)

install_data(
  'os-release',
  install_dir: get_option('prefix') / 'lib',
  rename: 'os-release',
)
