import CoreGraphics
import Foundation
import ImageIO

struct Options {
    var input: String?
    var output: String?
}

func usage() -> Never {
    fputs("Usage: make_dock_icon.swift --input INPUT.png --output OUTPUT.png\n", stderr)
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

let width = image.width
let height = image.height
let colorSpace = CGColorSpaceCreateDeviceRGB()
var pixels = [UInt8](repeating: 0, count: width * height * 4)
guard let context = CGContext(
    data: &pixels,
    width: width,
    height: height,
    bitsPerComponent: 8,
    bytesPerRow: width * 4,
    space: colorSpace,
    bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
) else {
    fatalError("Unable to create rendering context")
}

let canvas = CGRect(x: 0, y: 0, width: width, height: height)
let cornerRadius = CGFloat(min(width, height)) * 0.22
context.setFillColor(CGColor(gray: 1, alpha: 1))
context.addPath(CGPath(
    roundedRect: canvas,
    cornerWidth: cornerRadius,
    cornerHeight: cornerRadius,
    transform: nil
))
context.fillPath()
context.draw(image, in: canvas)

guard let result = context.makeImage() else { fatalError("Unable to render output") }
let outputURL = URL(fileURLWithPath: output) as CFURL
guard let destination = CGImageDestinationCreateWithURL(outputURL, "public.png" as CFString, 1, nil) else {
    fatalError("Unable to create output")
}
CGImageDestinationAddImage(destination, result, nil)
guard CGImageDestinationFinalize(destination) else { fatalError("Unable to write output") }
print("Wrote \(output)")
