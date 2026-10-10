# One GRUB image with everything it reads built in: the modules, the menu
# and the theme (nixos/branding), in a memdisk, so GRUB needs nothing of its
# own from any filesystem. Plain data, called by grub.nix (the EFI program
# on the ESP), bios.nix (the core image in the BIOS boot partition) and the
# installer's iso.nix (its El Torito image and its EFI program).
#
# Every one of them draws the same theme and writes to the serial port too,
# from the lines put in front of its menu here.
#
# The early config runs before the menu, while $root is still the device
# GRUB was started from: the prefix has no device so that it is.
{
  pkgs,
  # grub-mkimage's -O: i386-pc, i386-pc-eltorito, x86_64-efi or arm64-efi.
  format,
  # The file under $out.
  output,
  menu,
  early,
  modules,
  art,
}:

let
  bios = format == "i386-pc" || format == "i386-pc-eltorito";
  dir = if bios then "${pkgs.grub2}/lib/grub/i386-pc" else "${pkgs.grub2_efi}/lib/grub/${format}";

  # The screen's own mode on UEFI. On a BIOS, where VBE's "auto" is often a
  # mode the screen cannot show, 1024x768 first, and Linux keeps it as its
  # framebuffer, so Plymouth draws on it from the initrd (splash.nix).
  # Without a mode GRUB can set, the menu is text. The serial port, where
  # there is one, is how a VM test or a machine without a screen reads it.
  preamble = pkgs.writeText "grub-theme.cfg" ''
    ${
      if bios then
        ''
          set gfxmode=1024x768,auto
          set gfxpayload=keep
        ''
      else
        ''
          set gfxmode=auto
        ''
    }
    for font in (memdisk)/theme/*.pf2; do
      loadfont "$font"
    done
    if terminal_output gfxterm; then
      background_color "${art.palette.ground}"
      set theme=(memdisk)/theme/theme.txt
    fi
    if serial --unit=0 --speed=115200; then
      terminal_input --append serial
      terminal_output --append serial
    fi
  '';

  graphics = [
    "font"
    "gfxterm"
    "gfxterm_background"
    "gfxmenu"
    "png"
  ]
  ++ (
    if bios then
      [
        "vbe"
        "vga"
      ]
    else
      [ "efi_gop" ]
  );
in
pkgs.runCommand "losos-grub-${format}"
  {
    nativeBuildInputs = [ pkgs.buildPackages.grub2 ];
    passthru = { inherit menu; };
  }
  ''
    mkdir -p memdisk/theme $out
    cat ${preamble} ${menu} > memdisk/grub.cfg
    # The menu fails at boot, not here, unless it is checked here.
    grub-script-check memdisk/grub.cfg
    cp ${art}/grub/* memdisk/theme/
    tar -C memdisk -cf memdisk.tar grub.cfg theme
    cat > early.cfg <<'EOF'
    ${early}
    EOF
    grub-mkimage -O ${format} -d ${dir} -m memdisk.tar -c early.cfg -p / \
      -o $out/${output} ${toString (modules ++ graphics)}
  ''
