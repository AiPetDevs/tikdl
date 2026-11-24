import SwiftUI

/// Colours and sizes of the bar (Lab's dark palette). Change them here to fit your app.
public enum TBTheme {
    public static var field = Color(red: 0.165, green: 0.173, blue: 0.188)   // the empty pill
    public static var fill = Color(red: 0.32, green: 0.36, blue: 0.42)       // a filled share, idle
    public static var fillRun = Color(red: 0.227, green: 0.396, blue: 0.62)  // going
    public static var fillDone = Color(red: 0.216, green: 0.443, blue: 0.302)
    public static var fillFail = Color(red: 0.604, green: 0.294, blue: 0.294)
    public static var fg = Color(red: 0.90, green: 0.906, blue: 0.918)
    public static var dim = Color(red: 0.431, green: 0.439, blue: 0.467)
    public static var line = Color.white.opacity(0.07)
    public static var line2 = Color.white.opacity(0.12)
    public static var acc = Color(red: 0.478, green: 0.635, blue: 0.863)
    public static var ok = Color(red: 0.45, green: 0.75, blue: 0.55)
    public static var warn = Color(red: 0.851, green: 0.69, blue: 0.353)
    public static var bad = Color(red: 0.878, green: 0.478, blue: 0.478)
    public static var f11: CGFloat = 11, f13: CGFloat = 13
    public static var hPill: CGFloat = 22, icon: CGFloat = 16
    public static var cardWidth: CGFloat = 400, listMaxHeight: CGFloat = 360
}

// MARK: - Pill: a bar with its text inside (unknown = «—», never a fake 0)

public struct ProgressPill: View {
    var fraction: Double?
    var text: String
    var tint: Color?
    public init(fraction: Double?, text: String, tint: Color? = nil) { self.fraction = fraction; self.text = text; self.tint = tint }
    public var body: some View {
        ZStack(alignment: .leading) {
            Capsule().fill(TBTheme.field)
            // No animation on the value: it changes many times a second, and each animation frame re-lays out the
            // whole window (in Lab: CPU 9 % with it, 2 % without, measured).
            GeometryReader { g in
                Capsule().fill(tint ?? TBTheme.fill).frame(width: g.size.width * max(0, min(1, fraction ?? 0)))
            }
            HStack { Spacer(minLength: 0); Text(text).foregroundStyle(TBTheme.fg).lineLimit(1) }
                .font(.system(size: TBTheme.f13).monospacedDigit()).padding(.horizontal, 10)
        }
        .frame(height: TBTheme.hPill).frame(maxWidth: .infinity)
        .clipShape(Capsule())
    }
}

// MARK: - Ring: the transfer next to its file, as in Finder

public struct ProgressRing: View {
    public enum Mode { case going, waiting, paused, failed }
    var fraction: Double?
    var mode: Mode
    var size: CGFloat
    public init(fraction: Double?, mode: Mode, size: CGFloat = TBTheme.icon) { self.fraction = fraction; self.mode = mode; self.size = size }
    public var body: some View {
        ZStack {
            switch mode {
            case .going:
                Circle().stroke(TBTheme.line2, lineWidth: 2)
                // unknown size: a short arc, not an endless spinner
                Circle().trim(from: 0, to: max(0.04, min(1, fraction ?? 0.12)))
                    .stroke(TBTheme.acc, style: StrokeStyle(lineWidth: 2, lineCap: .round)).rotationEffect(.degrees(-90))
            case .waiting:
                Circle().stroke(TBTheme.dim, style: StrokeStyle(lineWidth: 1.5, dash: [2, 2.5]))
            case .paused:
                Circle().stroke(TBTheme.warn, lineWidth: 1.5)
                Image(systemName: "pause.fill").font(.system(size: size * 0.45, weight: .bold)).foregroundStyle(TBTheme.warn)
            case .failed:
                Circle().stroke(TBTheme.bad, lineWidth: 1.5)
                Image(systemName: "exclamationmark").font(.system(size: size * 0.55, weight: .bold)).foregroundStyle(TBTheme.bad)
            }
        }
        .frame(width: size, height: size)
    }
}

extension TransferJob {
    public var ringMode: ProgressRing.Mode {
        switch state {
        case .running, .done: return .going
        case .queued: return .waiting
        case .frozen, .paused: return .paused
        case .failed: return .failed
        }
    }
}

/// The ring of one job (put it next to the faded row of the file that is arriving).
public struct TransferRing: View {
    @ObservedObject var job: TransferJob
    public init(job: TransferJob) { self.job = job }
    public var body: some View { ProgressRing(fraction: job.fraction, mode: job.ringMode) }
}

// MARK: - Small parts

/// An icon button; with `arm` it is destructive: the first press arms it (red, the warning shown), the second runs.
public struct IconButton: View {
    var icon: String, help: String, arm: String?
    var action: () -> Void
    @State private var hover = false
    @State private var armed = false
    public init(icon: String, help: String, arm: String? = nil, action: @escaping () -> Void) {
        self.icon = icon; self.help = help; self.arm = arm; self.action = action
    }
    public var body: some View {
        Button {
            if arm != nil, !armed { armed = true; DispatchQueue.main.asyncAfter(deadline: .now() + 4) { armed = false }; return }
            armed = false; action()
        } label: {
            Image(systemName: icon).font(.system(size: TBTheme.icon - 1))
                .foregroundStyle(armed ? .black : hover ? TBTheme.fg : TBTheme.dim)
                .frame(width: 24, height: 24)
                .background(RoundedRectangle(cornerRadius: 5).fill(armed ? TBTheme.bad : hover ? TBTheme.line2 : .clear))
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain).onHover { hover = $0 }.help(armed ? (arm ?? help) : help).accessibilityLabel(help)
    }
}

struct StateTag: View {
    var text: String, color: Color
    var body: some View {
        Text(text).font(.system(size: TBTheme.f11)).foregroundStyle(color)
            .padding(.horizontal, 7).padding(.vertical, 1)
            .overlay(Capsule().stroke(color.opacity(0.5), lineWidth: 1))
    }
}

// MARK: - The card of one transfer: what, how much of how much, speed, ⏸ ▶ ✕

public struct TransferRow: View {
    @ObservedObject var job: TransferJob
    let queue: TransferQueue
    public init(job: TransferJob, queue: TransferQueue) { self.job = job; self.queue = queue }

    /// «0 байт», not «Zero KB».
    static func b(_ n: Int64) -> String {
        let f = ByteCountFormatter(); f.countStyle = .file; f.allowsNonnumericFormatting = false
        return f.string(fromByteCount: n)
    }

    public var body: some View {
        let s = job.spec
        VStack(alignment: .leading, spacing: 8) {
            HStack(spacing: 8) {
                Image(systemName: s.kind == .sync ? "arrow.triangle.2.circlepath" : s.kind == .move ? "scissors" : "doc.on.doc")
                    .font(.system(size: TBTheme.icon - 2)).foregroundStyle(TBTheme.dim)
                Text(title).foregroundStyle(TBTheme.fg).lineLimit(1).truncationMode(.middle)
                    .help(job.current.isEmpty ? s.name : "\(s.name) — сейчас: \(job.current)")
                Spacer(minLength: 8)
                tag
            }
            HStack(spacing: 8) {
                ProgressPill(fraction: job.fraction ?? (job.done > 0 ? 0 : nil), text: amount,
                             tint: isFailed ? TBTheme.fillFail : job.state == .running ? TBTheme.fillRun : job.state == .done ? TBTheme.fillDone : nil)
                controls
            }
        }
        .font(.system(size: TBTheme.f13))
        .padding(12)
    }

    private var title: String {
        let s = job.spec
        if s.kind == .sync { return s.name }
        let d = s.direction.isEmpty ? "" : " \(s.direction)"
        return "\(s.kind == .move ? "Перенос" : "Копия") «\(s.name)»\(d)"
    }

    private var isFailed: Bool { if case .failed = job.state { return true }; return false }

    private var amount: String {
        var t = job.spec.total.map { "\(Self.b(min(job.done, $0))) из \(Self.b($0))" } ?? (job.done > 0 ? "передано \(Self.b(job.done))" : "")
        if job.state == .running, !job.speed.isEmpty { t += " · \(job.speed)" }
        return t.isEmpty ? "—" : t
    }

    @ViewBuilder private var tag: some View {
        switch job.state {
        case .queued: StateTag(text: "в очереди", color: TBTheme.dim)
        case .running: EmptyView()
        case .frozen: StateTag(text: "пауза", color: TBTheme.warn).help("Заморожена: связь открыта, ▶ продолжит с того же места")
        case .paused(let why): StateTag(text: "на паузе", color: TBTheme.warn).help("\(why). Недокачанное сохранено — ▶ дошлёт только остаток")
        case .done: StateTag(text: "готово", color: TBTheme.ok)
        case .failed(let why): StateTag(text: "сбой", color: TBTheme.bad).help(why)
        }
    }

    @ViewBuilder private var controls: some View {
        switch job.state {
        case .running, .queued:
            IconButton(icon: "pause.fill", help: "Пауза — продолжить можно когда угодно, даже после перезапуска") { queue.pause(job) }
        case .frozen, .paused, .failed:
            IconButton(icon: "play.fill", help: "Продолжить — дошлёт только недостающее") { queue.resume(job) }
        case .done: EmptyView()
        }
        if job.state != .done {
            IconButton(icon: "xmark", help: "Отменить передачу",
                       arm: "Нажмите ещё раз: передача отменится, недокачанное удалится (дошедшие целиком файлы останутся)") { queue.cancel(job) }
        }
    }
}

/// Several cards one under another (a popover or a panel).
public struct TransferList: View {
    @ObservedObject var queue: TransferQueue
    public init(queue: TransferQueue) { self.queue = queue }
    public var body: some View {
        ScrollView {
            VStack(spacing: 0) {
                ForEach(Array(queue.jobs.enumerated()), id: \.element.id) { i, j in
                    if i > 0 { Rectangle().fill(TBTheme.line).frame(height: 1) }
                    TransferRow(job: j, queue: queue)
                }
            }
        }
        .frame(width: TBTheme.cardWidth).frame(maxHeight: TBTheme.listMaxHeight)
        .fixedSize(horizontal: false, vertical: true)
    }
}
