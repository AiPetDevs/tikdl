import Foundation
import Testing
@testable import TransferBar

struct ProgressParsing {
    @Test func oneFileGrowsThenFinishes() {
        var p = RsyncProgress()
        p.feed("video.mov\n     32768   0%    0.00kB/s    0:00:00\r")
        #expect(p.current == "video.mov" && p.done == 32768)
        p.feed("  31457280  50%   30.00MB/s    0:00:01\r  62914560 100%   30.00MB/s    0:00:02 (xfer#1, to-check=1/2)\n")
        #expect(p.done == 62914560 && p.speed == "30.00MB/s")
        // the second file adds to the first
        p.feed("notes.txt\n    1048576 100%    5.00MB/s    0:00:00 (xfr#2, to-chk=0/2)\n")
        #expect(p.done == 62914560 + 1048576 && p.current == "notes.txt")
    }

    @Test func chunksSplitAnywhere() {
        var p = RsyncProgress()
        for c in ["  10", "00  10%  1.00MB/s", "  0:00:01\r"] { p.feed(c) }
        #expect(p.done == 1000)
        p.feed("photos/\n")                        // a folder line is not a file name
        #expect(p.current == "")
    }
}

@MainActor struct QueueRuns {
    /// A real rsync between two temp folders: the job ends done and the file is there.
    @Test func copiesALocalFile() async throws {
        let base = FileManager.default.temporaryDirectory.appendingPathComponent("tb-test-\(UUID().uuidString)")
        let src = base.appendingPathComponent("a"), dst = base.appendingPathComponent("b")
        try FileManager.default.createDirectory(at: src, withIntermediateDirectories: true)
        try FileManager.default.createDirectory(at: dst, withIntermediateDirectories: true)
        FileManager.default.createFile(atPath: src.appendingPathComponent("x.bin").path, contents: Data(count: 3 << 20))
        defer { try? FileManager.default.removeItem(at: base) }
        let q = TransferQueue(saveKey: nil)
        var finished: TransferJob?
        q.onFinish = { finished = $0 }
        q.add(TransferSpec(kind: .copy, name: "x.bin", src: src.appendingPathComponent("x.bin").path, dst: dst.path + "/", total: 3 << 20))
        for _ in 0..<100 where finished == nil { try await Task.sleep(for: .milliseconds(50)) }
        #expect(finished?.state == .done && finished?.done == 3 << 20)
        #expect(FileManager.default.fileExists(atPath: dst.appendingPathComponent("x.bin").path))
    }
}

import SwiftUI
import AppKit

/// Draws the card in every state into a PNG (TB_SNAPSHOT=<path>): a look without opening a window.
@MainActor struct Snapshot {
    @Test func drawStates() throws {
        guard let out = ProcessInfo.processInfo.environment["TB_SNAPSHOT"] else { return }
        let q = TransferQueue(saveKey: nil)
        func job(_ name: String, _ kind: TransferSpec.Kind, _ st: TransferJob.State, _ done: Int64, _ total: Int64?, _ speed: String = "") -> TransferJob {
            let j = TransferJob(TransferSpec(kind: kind, name: name, src: "", dst: "", total: total, direction: "Mac → lab"))
            j.state = st; j.done = done; j.speed = speed
            return j
        }
        let jobs = [job("video.mov", .copy, .running, 38 << 20, 60 << 20, "8.00MB/s"),
                    job("photos", .move, .frozen, 12 << 20, 40 << 20),
                    job("dataset", .copy, .paused("нет связи"), 3 << 30, 10 << 30),
                    job("notes.txt", .copy, .queued, 0, 1 << 20),
                    job("Обновить справа", .sync, .failed("connection closed"), 5 << 20, nil),
                    job("report.pdf", .copy, .done, 2 << 20, 2 << 20)]
        let v = VStack(alignment: .leading, spacing: 0) {
            HStack(spacing: 14) { ForEach(jobs) { j in HStack(spacing: 6) { TransferRing(job: j); Text(j.spec.name).foregroundStyle(TBTheme.fg.opacity(0.5)) } } }
                .padding(12)
            ForEach(jobs) { j in Rectangle().fill(TBTheme.line).frame(height: 1); TransferRow(job: j, queue: q) }
        }
        .frame(width: 560).background(Color(red: 0.11, green: 0.11, blue: 0.12)).environment(\.colorScheme, .dark)
        let r = ImageRenderer(content: v); r.scale = 2
        let img = try #require(r.nsImage)
        let tiff = try #require(img.tiffRepresentation)
        let rep = try #require(NSBitmapImageRep(data: tiff))
        let png = try #require(rep.representation(using: .png, properties: [:]))
        try png.write(to: URL(fileURLWithPath: out))
    }
}
