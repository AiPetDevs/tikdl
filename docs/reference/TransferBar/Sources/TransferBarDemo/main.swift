import AppKit
import SwiftUI
import TransferBar

// Demo: copies generated files between two temporary folders on this Mac, slowed down (--bwlimit) so the bar,
// the ring, pause, resume and cancel can be seen. Run: swift run TransferBarDemo

@MainActor
final class Demo: ObservableObject {
    let queue = TransferQueue(saveKey: nil)       // nil: the demo does not keep its queue between launches
    let base = FileManager.default.temporaryDirectory.appendingPathComponent("TransferBarDemo")
    @Published var made = false

    func make() {
        let src = base.appendingPathComponent("from"), dst = base.appendingPathComponent("to")
        try? FileManager.default.removeItem(at: base)
        try? FileManager.default.createDirectory(at: src, withIntermediateDirectories: true)
        try? FileManager.default.createDirectory(at: dst, withIntermediateDirectories: true)
        for (name, mb) in [("video.mov", 60), ("photos", 0), ("notes.txt", 1)] {
            if mb == 0 {
                let d = src.appendingPathComponent(name)
                try? FileManager.default.createDirectory(at: d, withIntermediateDirectories: true)
                for i in 1...5 { FileManager.default.createFile(atPath: d.appendingPathComponent("img\(i).jpg").path, contents: Data(count: 8 << 20)) }
            } else {
                FileManager.default.createFile(atPath: src.appendingPathComponent(name).path, contents: Data(count: mb << 20))
            }
        }
        made = true
    }

    func copyAll() {
        if !made { make() }
        let src = base.appendingPathComponent("from").path, dst = base.appendingPathComponent("to").path
        for (name, total) in [("video.mov", Int64(60 << 20)), ("photos", Int64(40 << 20)), ("notes.txt", Int64(1 << 20))] {
            queue.add(TransferSpec(kind: .copy, name: name, src: "\(src)/\(name)", dst: "\(dst)/",
                                   extra: ["--bwlimit=8000"], total: total, direction: "from → to", cleanRoot: dst))
        }
    }
}

struct DemoView: View {
    @StateObject var demo = Demo()
    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            HStack {
                Button("Скопировать три примера") { demo.copyAll() }
                Text("медленно (8 МБ/с), чтобы было видно паузу и ✕").foregroundStyle(TBTheme.dim)
            }
            // the faded rows with rings, as the arriving files look in a folder
            VStack(alignment: .leading, spacing: 6) {
                ForEach(demo.queue.jobs) { j in
                    HStack(spacing: 8) {
                        TransferRing(job: j)
                        Text(j.spec.name).foregroundStyle(TBTheme.fg.opacity(0.5))
                    }
                }
            }
            Divider()
            TransferList(queue: demo.queue)
            Spacer()
        }
        .padding(20)
        .frame(minWidth: 460, minHeight: 420)
        .background(Color(red: 0.11, green: 0.11, blue: 0.12))
        .preferredColorScheme(.dark)
    }
}

final class AppDelegate: NSObject, NSApplicationDelegate {
    var window: NSWindow?
    func applicationDidFinishLaunching(_ n: Notification) {
        let w = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 480, height: 480),
                         styleMask: [.titled, .closable, .resizable], backing: .buffered, defer: false)
        w.title = "TransferBar demo"
        w.contentView = NSHostingView(rootView: DemoView())
        w.center(); w.makeKeyAndOrderFront(nil)
        window = w
        NSApp.activate(ignoringOtherApps: true)
    }
    func applicationShouldTerminateAfterLastWindowClosed(_ s: NSApplication) -> Bool { true }
}

let app = NSApplication.shared
let delegate = AppDelegate()
app.delegate = delegate
app.setActivationPolicy(.regular)
app.run()
