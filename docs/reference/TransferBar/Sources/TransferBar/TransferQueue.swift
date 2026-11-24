import Foundation
import Darwin

/// One transfer as it is saved: what, from where to where. Everything rides on rsync.
public struct TransferSpec: Codable, Identifiable, Equatable, Sendable {
    public enum Kind: String, Codable, Sendable { case copy, move, sync }
    public var id = UUID()
    public var kind: Kind
    public var name: String               // in words: a file, a folder, «Обновить справа»
    public var src: String, dst: String   // rsync arguments («host:/path» for the remote side)
    public var extra: [String] = []       // e.g. ["--delete-after"] for a mirror, ["--update"]
    public var total: Int64?              // bytes, when known in advance (else the pill shows «передано N»)
    public var direction: String = ""     // shown in the title, e.g. «Mac → lab»
    /// Where unfinished pieces lie (the target folder; its `.partial` subfolder is removed on ✕). Local only;
    /// for a remote target pass `TransferQueue.removeRemote`.
    public var cleanRoot: String = ""
    public var cleanIsRemote = false

    public init(kind: Kind, name: String, src: String, dst: String, extra: [String] = [], total: Int64? = nil,
                direction: String = "", cleanRoot: String = "", cleanIsRemote: Bool = false) {
        self.kind = kind; self.name = name; self.src = src; self.dst = dst; self.extra = extra
        self.total = total; self.direction = direction; self.cleanRoot = cleanRoot; self.cleanIsRemote = cleanIsRemote
    }
}

@MainActor
public final class TransferJob: ObservableObject, Identifiable {
    public enum State: Equatable { case queued, running, frozen, paused(String), done, failed(String) }
    nonisolated public let spec: TransferSpec
    nonisolated public var id: UUID { spec.id }
    @Published public var state: State = .queued
    @Published public var done: Int64 = 0
    @Published public var speed = ""
    @Published public internal(set) var current = ""
    public internal(set) var userPaused = false   // paused by the user (not by a lost link): stays paused
    var proc: Process?
    var stopping = false                          // we stopped it ourselves (pause «later» or ✕), not a failure
    var tries = 0
    public init(_ s: TransferSpec) { spec = s }

    /// 0…1, or nil when the total is unknown.
    public var fraction: Double? {
        guard let t = spec.total, t > 0 else { return nil }
        return min(1, Double(done) / Double(t))
    }
}

/// The queue: one rsync at a time, in order.
///
/// Pause has two depths. ⏸ freezes the running rsync (SIGSTOP to it and its ssh child): the link stays open and ▶
/// (SIGCONT) goes on from the same byte. «Later» (`stopAll`, e.g. the link dropped or the app quits) stops rsync; the
/// unfinished piece stays in a hidden `.partial` folder at the target, so the next run sends only the rest.
/// The queue is saved in UserDefaults: after a restart unfinished transfers come back paused.
@MainActor
public final class TransferQueue: ObservableObject {
    @Published public private(set) var jobs: [TransferJob] = []

    /// `rsync -e` value for remote paths, e.g. "ssh -o BatchMode=yes" (nil — local copies only).
    public var rsh: String?
    /// Is the other side reachable now? A failed run while offline waits instead of counting a try.
    public var isOnline: () -> Bool = { true }
    /// Called when a job finished (e.g. re-read the target folder, then `forget(job)`). Default: forget at once.
    public var onFinish: ((TransferJob) -> Void)?
    /// For `.move`: remove the original after the whole thing arrived (into the Trash, preferably).
    public var removeSource: ((TransferSpec) async -> Void)?
    /// For a remote target: run a shell command there (used to remove `.partial` on ✕).
    public var removeRemote: ((String) async -> Void)?
    /// A line for your log.
    public var log: (String) -> Void = { _ in }

    public static let partialDir = ".partial"
    private let saveKey: String?

    public init(saveKey: String? = "transferQueue") {
        self.saveKey = saveKey
        guard let k = saveKey, let d = UserDefaults.standard.data(forKey: k),
              let saved = try? JSONDecoder().decode([Saved].self, from: d) else { return }
        jobs = saved.map { s in
            let j = TransferJob(s.spec)
            // a mirror deletes by the folders as they are when it runs: after a restart it waits for the user's ▶
            let mirror = s.spec.extra.contains("--delete-after")
            j.userPaused = s.userPaused || mirror
            j.state = .paused(s.userPaused ? "на паузе" : mirror ? "зеркало — ▶, чтобы продолжить" : "после перезапуска")
            return j
        }
    }

    private struct Saved: Codable { var spec: TransferSpec; var userPaused: Bool }
    private func save() {
        guard let k = saveKey else { return }
        let keep = jobs.filter { $0.state != .done }.map { Saved(spec: $0.spec, userPaused: $0.userPaused) }
        UserDefaults.standard.set(try? JSONEncoder().encode(keep), forKey: k)
    }

    public var active: TransferJob? { jobs.first { $0.state == .running || $0.state == .frozen } }
    public var busy: Bool { jobs.contains { $0.state == .running || $0.state == .frozen || $0.state == .queued } }

    // MARK: commands

    public func add(_ s: TransferSpec) {
        jobs.append(TransferJob(s)); save()
        log("в очередь: \(s.kind.rawValue) \(s.name)")
        next()
    }

    /// ⏸: freeze the running one (instant ▶); a waiting one is just held.
    public func pause(_ j: TransferJob) {
        j.userPaused = true
        switch j.state {
        case .running:
            if let p = proc(j) { signal(p, SIGSTOP) }
            j.state = .frozen; j.speed = ""
        case .queued: j.state = .paused("на паузе")
        default: break
        }
        save(); next()
    }

    /// ▶: unfreeze, or run again — rsync sends only what is missing.
    public func resume(_ j: TransferJob) {
        j.userPaused = false
        if j.state == .frozen, let p = proc(j) { signal(p, SIGCONT); j.state = .running }
        else if case .paused = j.state { j.state = .queued; j.tries = 0 }
        else if case .failed = j.state { j.state = .queued; j.tries = 0 }
        save(); next()
    }

    /// «Later» for everything (lost link, app quits): stop rsync, keep the pieces.
    public func stopAll(_ why: String) {
        for j in jobs where j.state == .running || j.state == .frozen {
            j.stopping = true
            if let p = proc(j) { if j.state == .frozen { signal(p, SIGCONT) }; p.terminate() }
            j.state = .paused(why)
        }
        save()
    }

    /// ✕: stop and remove the unfinished pieces at the target (files that fully arrived stay).
    public func cancel(_ j: TransferJob) {
        if let p = proc(j) { j.stopping = true; if j.state == .frozen { signal(p, SIGCONT) }; p.terminate() }
        jobs.removeAll { $0.id == j.id }; save()
        Task { await cleanPieces(j.spec) }
        log("отменена: \(j.spec.name)")
        next()
    }

    public func forget(_ j: TransferJob) { jobs.removeAll { $0.id == j.id } }

    /// The other side is back: go on with what the link (not the user) paused.
    public func linkUp() {
        for j in jobs where !j.userPaused { if case .paused = j.state { j.state = .queued; j.tries = 0 } }
        next()
    }

    // MARK: running

    private func proc(_ j: TransferJob) -> Process? { j.proc?.isRunning == true ? j.proc : nil }

    private func signal(_ p: Process, _ sig: Int32) {
        kill(p.processIdentifier, sig)
        // rsync's own ssh child too, so nothing moves while frozen
        let k = Process(); k.executableURL = URL(fileURLWithPath: "/usr/bin/pkill")
        k.arguments = [sig == SIGSTOP ? "-STOP" : "-CONT", "-P", String(p.processIdentifier)]
        try? k.run()
    }

    private func next() {
        guard active == nil, isOnline(), let j = jobs.first(where: { $0.state == .queued }) else { return }
        run(j)
    }

    private func run(_ j: TransferJob) {
        var args = ["-a", "--partial", "--partial-dir=\(Self.partialDir)", "--exclude=\(Self.partialDir)/", "--progress"]
        if let rsh { args = ["-e", rsh] + args }
        args += j.spec.extra + [j.spec.src, j.spec.dst]
        let p = Process()
        p.executableURL = URL(fileURLWithPath: "/usr/bin/rsync")
        p.arguments = args
        let out = Pipe(), err = Pipe()
        p.standardOutput = out; p.standardError = err; p.standardInput = FileHandle.nullDevice
        j.state = .running; j.done = 0; j.stopping = false; j.speed = ""; j.current = ""
        j.proc = p
        var prog = RsyncProgress()
        out.fileHandleForReading.readabilityHandler = { [weak j] h in
            let d = h.availableData
            guard !d.isEmpty else { return }
            let chunk = String(decoding: d, as: UTF8.self)
            // DispatchQueue.main keeps the chunks in order (separate Tasks need not run in order)
            DispatchQueue.main.async { MainActor.assumeIsolated {
                guard let j else { return }
                prog.feed(chunk)
                j.done = prog.done; j.speed = prog.speed; j.current = prog.current
            } }
        }
        var errText = ""
        err.fileHandleForReading.readabilityHandler = { h in
            let d = h.availableData
            guard !d.isEmpty else { return }
            let s = String(decoding: d, as: UTF8.self)
            DispatchQueue.main.async { MainActor.assumeIsolated { errText += s } }
        }
        p.terminationHandler = { [weak self, weak j] pr in
            let status = pr.terminationStatus
            // after the output chunks already queued on main, so the last progress and error text are in
            DispatchQueue.main.async { MainActor.assumeIsolated {
                out.fileHandleForReading.readabilityHandler = nil; err.fileHandleForReading.readabilityHandler = nil
                // a stopped run's late end must not touch a newer run of the same job
                guard let self, let j, j.proc === pr else { return }
                let text = errText
                Task { await self.ended(j, status: status, err: text) }
            } }
        }
        do { try p.run() } catch { j.state = .failed(error.localizedDescription); j.proc = nil; save(); next(); return }
        log("идёт: \(j.spec.name)")
    }

    private func ended(_ j: TransferJob, status: Int32, err: String) async {
        j.proc = nil
        if j.stopping { j.stopping = false; next(); return }
        if status == 0 {
            if j.spec.kind == .move { await removeSource?(j.spec) }
            j.state = .done; j.speed = ""
            if let t = j.spec.total { j.done = t }
            log("готово: \(j.spec.name)")
            if let f = onFinish { f(j) } else { forget(j) }
        } else if !isOnline() {
            j.state = .paused("нет связи — продолжу, когда вернётся")
        } else if j.tries < 3 {
            j.tries += 1; j.state = .queued        // the link blinked: go on from the piece already there
            log("обрыв \(j.spec.name) (код \(status)), попытка \(j.tries)")
        } else {
            j.state = .failed(err.split(separator: "\n").last.map(String.init) ?? "код \(status)")
            log("не прошла \(j.spec.name): \(err.suffix(300))")
        }
        save(); next()
    }

    private func cleanPieces(_ s: TransferSpec) async {
        guard !s.cleanRoot.isEmpty else { return }
        let dir = (s.cleanRoot as NSString).appendingPathComponent(Self.partialDir)
        if s.cleanIsRemote { await removeRemote?("rm -rf -- '\(dir.replacingOccurrences(of: "'", with: "'\\''"))'") }
        else { try? FileManager.default.removeItem(atPath: dir) }
    }
}
