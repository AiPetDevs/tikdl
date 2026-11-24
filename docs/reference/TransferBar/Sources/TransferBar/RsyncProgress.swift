import Foundation

/// Reads `rsync --progress` output and keeps the byte count of the whole run.
///
/// rsync prints one progress line per file and rewrites it with `\r` while the file goes:
///     `   1048576  45%   12.34MB/s    0:00:02`
/// and ends it with `\n` plus `(xfer#3, to-check=7/12)` (`xfr#` in newer rsync) when the file is done.
/// Any other line is a file name. Pure value type: feed it raw chunks in order, read the fields.
public struct RsyncProgress: Equatable {
    public private(set) var done: Int64 = 0      // bytes of this run: finished files + the current one
    public private(set) var speed = ""            // as rsync says it, e.g. «12.34MB/s»
    public private(set) var current = ""          // the file going now (last path component)
    private var finished: Int64 = 0
    private var buf = ""

    public init() {}

    public mutating func feed(_ chunk: String) {
        buf += chunk
        while let r = buf.firstIndex(where: { $0 == "\r" || $0 == "\n" }) {
            let line = String(buf[..<r]).trimmingCharacters(in: .whitespaces)
            buf.removeSubrange(...r)
            guard !line.isEmpty else { continue }
            let f = line.split(separator: " ", omittingEmptySubsequences: true)
            if f.count >= 3, let b = Int64(f[0]), f[1].hasSuffix("%") {
                speed = String(f[2])
                if line.contains("xfer#") || line.contains("xfr#") { finished += b; done = finished }
                else { done = finished + b }
            } else if !line.hasSuffix("/") {
                current = (line as NSString).lastPathComponent
            }
        }
    }
}
