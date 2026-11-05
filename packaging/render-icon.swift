// Render the Finder/Dock icon macOS actually shows for an app bundle (reflects IconServices treatment).
import AppKit
let args = CommandLine.arguments
let img = NSWorkspace.shared.icon(forFile: args[1])
img.size = NSSize(width: 512, height: 512)
guard let tiff = img.tiffRepresentation, let rep = NSBitmapImageRep(data: tiff),
      let png = rep.representation(using: .png, properties: [:]) else { exit(1) }
try! png.write(to: URL(fileURLWithPath: args[2]))
print("wrote", args[2])
