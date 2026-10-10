# The boot pictures, drawn at build time from salmon.png, the logo LosOS
# boots behind (CREDITS.md), on the dark palette derisk and LosOS's WebUI
# share (mcsapi-theme's derisk-dark tokens), so replacing salmon.png
# replaces every one of them. Plain data, called by grub.nix, splash.nix and
# the installer's iso.nix:
#
#   grub/       GRUB's theme: theme.txt, the salmon, the fonts it names and
#               the highlight behind the selected entry
#   splash.bmp  the UKI's .splash, which systemd-stub centres on the screen
#               while the kernel and initrd load, between GRUB and Plymouth
#   logo.png    the salmon alone, for the Plymouth theme (splash.nix), which
#               scales it to the screen
#
# `name` is what the pictures call the system: "LosOS Desktop", or on the
# installer's medium "LosOS Desktop installer".
{ pkgs, name }:

let
  palette = {
    ground = "#0a0e12";
    ink = "#e2e8ee";
    muted = "#94a2b0";
    accent = "#48b3c0";
  };
  inherit (palette)
    ground
    ink
    muted
    accent
    ;

  # The logo without its transparent margin, scaled to fit a box of that
  # size and centred in it, so a logo of another shape lands in the same
  # place.
  fit =
    box: "${./salmon.png} -trim +repage -resize ${box} -background none -gravity center -extent ${box}";

  fonts = "${pkgs.dejavu_fonts}/share/fonts/truetype";
  bold = "${fonts}/DejaVuSans-Bold.ttf";
  regular = "${fonts}/DejaVuSans.ttf";
  # The console GRUB writes "Booting" to after the menu lays text out on a
  # grid, so it gets a font whose letters are all one width.
  mono = "${fonts}/DejaVuSansMono.ttf";

  # One block in the middle of the screen, whatever its size: the salmon,
  # the name under it, the menu, and the countdown. Every top is an offset
  # from the middle in pixels, because GRUB draws the pictures and the fonts
  # at their own size and only positions scale. GRUB picks the screen's own
  # mode on UEFI and 1024x768 on a BIOS (grub.cfg), and the block fits
  # either.
  theme = pkgs.writeText "theme.txt" ''
    title-text: ""
    desktop-color: "${ground}"
    terminal-font: "DejaVu Sans Mono Regular 16"

    + image {
      left = 50%-150
      top = 50%-290
      file = "salmon.png"
    }

    + label {
      left = 0
      width = 100%
      top = 50%-82
      height = 40
      align = "center"
      text = "${name}"
      color = "${ink}"
      font = "DejaVu Sans Bold 28"
    }

    + boot_menu {
      left = 50%-270
      width = 540
      top = 50%-8
      height = 200
      item_font = "DejaVu Sans Regular 18"
      selected_item_font = "DejaVu Sans Bold 18"
      item_color = "${ink}"
      selected_item_color = "${ground}"
      selected_item_pixmap_style = "select_*.png"
      item_height = 34
      item_padding = 14
      item_spacing = 6
      icon_width = 0
      icon_height = 0
      item_icon_space = 0
      scrollbar = false
    }

    + label {
      left = 0
      width = 100%
      top = 50%+212
      height = 24
      align = "center"
      id = "__timeout__"
      text = "Starting in %d s"
      color = "${muted}"
      font = "DejaVu Sans Regular 14"
    }
  '';
in
pkgs.runCommand "losos-boot-art"
  {
    nativeBuildInputs = [
      pkgs.buildPackages.imagemagick
      pkgs.buildPackages.grub2
    ];
    passthru = { inherit palette; };
  }
  ''
    mkdir -p $out/grub

    magick ${fit "300x200"} -strip PNG32:$out/grub/salmon.png

    # The highlight behind the selected entry, as GRUB's nine-piece box.
    for piece in c n s e w ne nw se sw; do
      magick -size 4x4 xc:'${accent}' PNG24:$out/grub/select_$piece.png
    done

    # Latin, Latin-1 and Latin Extended-A, enough for the names and the
    # countdown in every language GRUB is likely to be asked for.
    font() {
      grub-mkfont -s "$2" -r 0x20-0x17F -o "$out/grub/$3.pf2" "$1"
    }
    font ${regular} 14 regular-14
    font ${mono} 16 mono-16
    font ${regular} 18 regular-18
    font ${bold} 18 bold-18
    font ${bold} 28 bold-28
    cp ${theme} $out/grub/theme.txt

    # systemd-stub centres this on a screen it clears to black, so the
    # ground is black and nothing frames it.
    magick -size 480x320 xc:black \
      \( ${fit "300x200"} \) -gravity north -geometry +0+20 -composite \
      -font ${bold} -fill '${ink}' -pointsize 30 -annotate +0+236 '${name}' \
      -type TrueColor BMP3:$out/splash.bmp

    # Large enough that the Plymouth theme only ever scales it down.
    magick ${./salmon.png} -trim +repage -resize 640x440 -strip PNG32:$out/logo.png
  ''
