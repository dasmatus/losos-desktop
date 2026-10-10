# Where the boot pictures come from

## salmon.png

The logo LosOS uses: a live coho salmon (*Oncorhynchus kisutch*), cut out
of the photo "Coho salmon" by NOAA Fisheries, on the coho salmon page of
<https://www.fisheries.noaa.gov/species/coho-salmon>. It is the same file as
`admin-ui/themes/brand/salmon.png` in [LosOS](https://github.com/dasmatus/losos),
copied unchanged, so the desktop boots behind the same fish as the box.

NOAA's website policy says images created by NOAA are not subject to
copyright in the United States and may be used without permission, and asks
for "NOAA" to be credited as the source. Photo: NOAA Fisheries. Its use here
does not mean that NOAA endorses LosOS.

The cut-out is the photo with the river bed removed, turned 35 degrees so
the fish rises from left to right, and scaled into a 512 px square. Every
picture `default.nix` draws from it is made at build time.
