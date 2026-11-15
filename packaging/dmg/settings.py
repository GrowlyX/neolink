# dmgbuild settings for Neolink.dmg; driven by scripts/dmg.sh.
#
# dmgbuild writes the Finder layout (.DS_Store, background alias, volume icon) directly,
# so the image builds headless on CI without scripting Finder.
#
# Geometry is shared with packaging/dmg/background.swift: 640x400 pt content, 112 pt icons
# centred at (160, 186) and (480, 186). Change both together.
import os.path

app = defines["app"]  # noqa: F821  (injected by dmgbuild -D)
background_tiff = defines["background"]  # noqa: F821
icns = defines["icns"]  # noqa: F821

format = "ULFO"  # LZFSE, read-only; any macOS we support (14+) mounts it
filesystem = "HFS+"
files = [app]
symlinks = {"Applications": "/Applications"}
hide_extension = [os.path.basename(app)]
icon = icns

background = background_tiff
show_status_bar = False
show_tab_view = False
show_toolbar = False
show_pathbar = False
show_sidebar = False
sidebar_width = 0

# 400 pt of content plus the 28 pt title bar.
window_rect = ((200, 120), (640, 428))
default_view = "icon-view"
show_icon_preview = True

arrange_by = None
icon_size = 112
text_size = 12
label_pos = "bottom"
icon_locations = {
    os.path.basename(app): (160, 186),
    "Applications": (480, 186),
}
