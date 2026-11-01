import FolioTokens
import Foundation

// Usage: folio-tokengen <tokens.json> <FolioKit package root>
let arguments = CommandLine.arguments
guard arguments.count == 3 else {
    FileHandle.standardError.write(Data("usage: folio-tokengen <tokens.json> <package root>\n".utf8))
    exit(64)
}

do {
    let data = try Data(contentsOf: URL(filePath: arguments[1]))
    let output = try TokenGenerator.generate(from: data)
    try output.write(toPackageAt: URL(filePath: arguments[2], directoryHint: .isDirectory))
    print("Wrote \(output.filesByPackagePath.count) files")
} catch {
    FileHandle.standardError.write(Data("folio-tokengen: \(error)\n".utf8))
    exit(1)
}
