# The Android NDK/HAL headers libhybris is compiled against, as Halium
# publishes them. One tree serves every Halium base from 11 to 16: the
# halium-11.0 through halium-16.0 branches all point at this commit. A device
# whose vendor HALs need an older ABI (Halium 9 or 10) pins its own.
{
  lib,
  stdenvNoCC,
  fetchgit,
}:

stdenvNoCC.mkDerivation {
  pname = "android-headers";
  version = "11.0.0-unstable-2026-06-06";

  src = fetchgit {
    url = "https://github.com/Halium/android-headers";
    rev = "217a38e92cc64a96b4d1ac7469f30827d84485d3";
    hash = "sha256-dYRlMs503R0jVSIYeHqReeoXySTVq/UlemhVRktIHj4=";
  };

  dontBuild = true;
  makeFlags = [ "PREFIX=${placeholder "out"}" ];

  meta = {
    description = "Android headers for building libhybris against a Halium vendor image";
    homepage = "https://github.com/Halium/android-headers";
    license = lib.licenses.asl20;
    platforms = lib.platforms.linux;
  };
}
