// Renders the drag-to-install background for Neolink.dmg.
//
//   swift packaging/dmg/background.swift <repo-root> <out-dir> <version>
//
// Writes background.png (1x) and background@2x.png; scripts/dmg.sh merges them into one
// HiDPI TIFF so Finder picks the right one on each display. Everything is drawn here from
// the brand SVGs and the bundled Geist font, so the art is reproducible and reviewable as code.
//
// Geometry is shared with packaging/dmg/settings.py: a 640x400 pt window, icons 112 pt, centred at
// (160, 186) and (480, 186). Change both together.

import AppKit
import CoreText

let args = CommandLine.arguments
guard args.count == 4 else {
    FileHandle.standardError.write("usage: background.swift <repo-root> <out-dir> <version>\n".data(using: .utf8)!)
    exit(2)
}
let root = URL(fileURLWithPath: args[1])
let outDir = URL(fileURLWithPath: args[2])
let version = args[3]

let W: CGFloat = 640, H: CGFloat = 400
let appCenter = CGPoint(x: 160, y: 186)
let appsCenter = CGPoint(x: 480, y: 186)
let iconSize: CGFloat = 112

for weight in ["Regular", "Medium", "SemiBold"] {
    let url = root.appendingPathComponent("crates/neolink-app/assets/fonts/Geist-\(weight).ttf")
    CTFontManagerRegisterFontsForURL(url as CFURL, .process, nil)
}
func geist(_ size: CGFloat, _ weight: String = "Regular") -> NSFont {
    NSFont(name: "Geist-\(weight)", size: size) ?? NSFont.systemFont(ofSize: size)
}

/// Load an SVG and recolour it by masking, so the black or white brand files can take any tint.
func tinted(_ path: String, _ color: NSColor, _ size: NSSize) -> NSImage {
    let src = NSImage(contentsOf: root.appendingPathComponent(path))!
    return NSImage(size: size, flipped: false) { rect in
        src.draw(in: rect)
        color.set()
        rect.fill(using: .sourceAtop)
        return true
    }
}

func render(scale: CGFloat) -> Data {
    let rep = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: Int(W * scale), pixelsHigh: Int(H * scale),
                               bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false,
                               colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0)!
    rep.size = NSSize(width: W, height: H)
    let ctx = NSGraphicsContext(bitmapImageRep: rep)!
    NSGraphicsContext.saveGraphicsState()
    NSGraphicsContext.current = ctx
    // Flip so y grows downward, matching Finder's icon coordinates.
    let cg = ctx.cgContext
    cg.translateBy(x: 0, y: H)
    cg.scaleBy(x: 1, y: -1)
    let flipped = NSGraphicsContext(cgContext: cg, flipped: true)
    NSGraphicsContext.current = flipped

    // Strictly black and white: flat black field, white marks, no tints, glows or shadows.
    let ink = NSColor.white
    NSColor.black.setFill()
    NSRect(x: 0, y: 0, width: W, height: H).fill()

    // Lockup, top centre.
    let lockupH: CGFloat = 20
    let lockupW = lockupH * 924 / 136
    tinted("crates/neolink-app/assets/brand/neolink-lockup-white.svg", ink,
           NSSize(width: lockupW, height: lockupH))
        .draw(in: NSRect(x: (W - lockupW) / 2, y: 40, width: lockupW, height: lockupH),
              from: .zero, operation: .sourceOver, fraction: 1, respectFlipped: true, hints: nil)

    // Arrow from the app to Applications: an evenly dotted trail and a chevron head.
    let x0 = appCenter.x + iconSize / 2 + 28, x1 = appsCenter.x - iconSize / 2 - 28
    ink.setFill()
    let pitch: CGFloat = 9
    var x = x0
    while x < x1 - 8 {
        NSBezierPath(ovalIn: NSRect(x: x - 1.5, y: appCenter.y - 1.5, width: 3, height: 3)).fill()
        x += pitch
    }
    let head = NSBezierPath()
    head.move(to: CGPoint(x: x1 - 8, y: appCenter.y - 7))
    head.line(to: CGPoint(x: x1, y: appCenter.y))
    head.line(to: CGPoint(x: x1 - 8, y: appCenter.y + 7))
    head.lineWidth = 2
    head.lineCapStyle = .round
    head.lineJoinStyle = .round
    ink.setStroke()
    head.stroke()

    // Caption. Hierarchy comes from size and weight alone.
    func centered(_ text: String, _ font: NSFont, _ y: CGFloat, kern: CGFloat = 0) {
        let s = NSAttributedString(string: text, attributes: [.font: font, .foregroundColor: ink, .kern: kern])
        s.draw(at: CGPoint(x: (W - s.size().width) / 2, y: y))
    }
    centered("Drag Neolink into Applications", geist(15, "Medium"), 300)
    centered("Then open it. Setup installs the DisplayLink driver and walks you through the rest.",
             geist(12), 324)

    // Hairline rule and a quiet footer.
    ink.setFill()
    NSRect(x: (W - 24) / 2, y: H - 46, width: 24, height: 1).fill()
    centered("v\(version)  ·  macOS 14 or later  ·  Universal", geist(10.5), H - 32, kern: 0.3)

    NSGraphicsContext.restoreGraphicsState()
    return rep.representation(using: .png, properties: [:])!
}

try! FileManager.default.createDirectory(at: outDir, withIntermediateDirectories: true)
try! render(scale: 1).write(to: outDir.appendingPathComponent("background.png"))
try! render(scale: 2).write(to: outDir.appendingPathComponent("background@2x.png"))
print("rendered \(outDir.path)/background{,@2x}.png")
