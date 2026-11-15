// Regenerates packaging/Neolink.icns from the brand mark.
//
//   swift packaging/icon.swift <repo-root>
//
// Black rounded square on the standard macOS icon grid (824 pt tile in a 1024 canvas),
// white Neolink mark, no gradient or shadow, matching brand/neolink-icon.svg.

import AppKit

let root = URL(fileURLWithPath: CommandLine.arguments.count > 1 ? CommandLine.arguments[1] : ".")
let mark = NSImage(contentsOf: root.appendingPathComponent("crates/neolink-app/assets/brand/neolink-mark-white.svg"))!
let iconset = FileManager.default.temporaryDirectory.appendingPathComponent("Neolink.iconset")
try? FileManager.default.removeItem(at: iconset)
try! FileManager.default.createDirectory(at: iconset, withIntermediateDirectories: true)

func render(_ px: Int) -> Data {
    let rep = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: px, pixelsHigh: px, bitsPerSample: 8,
                               samplesPerPixel: 4, hasAlpha: true, isPlanar: false,
                               colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0)!
    NSGraphicsContext.saveGraphicsState()
    NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: rep)
    let s = CGFloat(px) / 1024
    let tile = NSRect(x: 100 * s, y: 100 * s, width: 824 * s, height: 824 * s)
    NSColor.black.setFill()
    NSBezierPath(roundedRect: tile, xRadius: 185 * s, yRadius: 185 * s).fill()
    let m: CGFloat = 400 * s
    mark.draw(in: NSRect(x: (CGFloat(px) - m) / 2, y: (CGFloat(px) - m) / 2, width: m, height: m))
    NSGraphicsContext.restoreGraphicsState()
    return rep.representation(using: .png, properties: [:])!
}

for base in [16, 32, 128, 256, 512] {
    try! render(base).write(to: iconset.appendingPathComponent("icon_\(base)x\(base).png"))
    try! render(base * 2).write(to: iconset.appendingPathComponent("icon_\(base)x\(base)@2x.png"))
}
let out = root.appendingPathComponent("packaging/Neolink.icns").path
let p = Process()
p.executableURL = URL(fileURLWithPath: "/usr/bin/iconutil")
p.arguments = ["-c", "icns", iconset.path, "-o", out]
try! p.run()
p.waitUntilExit()
print("wrote \(out)")
