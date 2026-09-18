#!/usr/bin/env swift
//
// Builds a Dock icon master that matches the geometry of
// Assets/DeepSeekHarnessIcon-Prepared-1024.png:
//
//   * 1024x1024 canvas, fully transparent outside the tile
//   * tile offset by 89pt on every side (846pt square, the macOS safe area)
//   * continuous rounded corners with a 0.22 * side radius
//
// Usage:
//   xcrun swift scripts/make_prepared_dock_icon.swift \
//     --input SOURCE.jpg --output Assets/DeepSeekHarnessIcon-X-Prepared-1024.png
import CoreGraphics
import Foundation
import ImageIO

struct Options {
    var input: String?
    var output: String?
}

func usage() -> Never {
    fputs("Usage: make_prepared_dock_icon.swift --input SOURCE --output OUTPUT.png\n", stderr)
    exit(2)
}

var options = Options()
var arguments = Array(CommandLine.arguments.dropFirst())
var index = 0
while index < arguments.count {
    switch arguments[index] {
    case "--input":
        index += 1
        guard index < arguments.count else { usage() }
        options.input = arguments[index]
    case "--output":
        index += 1
        guard index < arguments.count else { usage() }
        options.output = arguments[index]
    default:
        usage()
    }
    index += 1
}

guard let input = options.input, let output = options.output else { usage() }
let inputURL = URL(fileURLWithPath: input) as CFURL
guard let source = CGImageSourceCreateWithURL(inputURL, nil),
      let image = CGImageSourceCreateImageAtIndex(source, 0, nil) else {
    fatalError("Unable to read \(input)")
}

let canvasSide = 1024
let tileSide = 846
let tileOrigin = (canvasSide - tileSide) / 2
let cornerRadius = CGFloat(tileSide) * 0.22

let colorSpace = CGColorSpaceCreateDeviceRGB()
guard let context = CGContext(
    data: nil,
    width: canvasSide,
    height: canvasSide,
    bitsPerComponent: 8,
    bytesPerRow: 0,
    space: colorSpace,
    bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
) else {
    fatalError("Unable to create rendering context")
}

let tile = CGRect(x: tileOrigin, y: tileOrigin, width: tileSide, height: tileSide)
context.saveGState()
context.addPath(CGPath(
    roundedRect: tile,
    cornerWidth: cornerRadius,
    cornerHeight: cornerRadius,
    transform: nil
))
context.clip()
context.interpolationQuality = .high
context.draw(image, in: tile)
context.restoreGState()

guard let result = context.makeImage() else { fatalError("Unable to render output") }
let outputURL = URL(fileURLWithPath: output) as CFURL
guard let destination = CGImageDestinationCreateWithURL(outputURL, "public.png" as CFString, 1, nil) else {
    fatalError("Unable to create output")
}
CGImageDestinationAddImage(destination, result, nil)
guard CGImageDestinationFinalize(destination) else { fatalError("Unable to write \(output)") }
print("Wrote \(output)")
