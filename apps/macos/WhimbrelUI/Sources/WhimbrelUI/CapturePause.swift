import Foundation

// The timed pause of work memory (docs/ui-redesign/screens/home.md, "Timed pause for capture"), as one
// pure decision. The app keeps `resumeAt` on the device, asks this type what to do for each event and
// does what the answer says. Nothing here reads a clock, a timer or storage: `now` comes in.

/// The three states of work memory. "Paused" is on hold with a stored time to come back.
public enum CapturePauseState: Equatable, Sendable { case on, paused, off }

/// The pause choices the menus offer.
public enum CapturePauseChoice: CaseIterable, Sendable {
    case fifteenMinutes, oneHour, untilTomorrow
}

/// Everything that can happen to the state.
public enum CapturePauseEvent: Equatable, Sendable {
    case pause(CapturePauseChoice)
    case resumeNow
    case turnOff
    case turnOn
    /// The timer fired. Compared against the wall clock, never trusted.
    case timeReached
    /// The app launched, with `resumeAt` read from storage.
    case appStarted
    case wokeFromSleep
    case deleteEverything
}

/// What the app does to capture itself. Whatever else gates capture (Accessibility, the recording gate,
/// no-store mode) still applies when the app acts on `.start`.
public enum CapturePauseAction: Equatable, Sendable { case none, start, stop }

public struct CapturePauseResult: Equatable, Sendable {
    public var state: CapturePauseState
    public var resumeAt: Date?
    public var action: CapturePauseAction
}

public enum CapturePauseModel {
    /// The hour "until tomorrow" ends, local time.
    public static let tomorrowHour = 4

    /// When a pause started at `now` ends. "Until tomorrow" is 4:00 AM local on the next calendar date, except between
    /// midnight and 4:00 AM, which still counts as the night before: then it is 4:00 AM the same date.
    public static func resumeTime(for choice: CapturePauseChoice, now: Date, calendar: Calendar = .current) -> Date {
        switch choice {
        case .fifteenMinutes: return now.addingTimeInterval(15 * 60)
        case .oneHour: return now.addingTimeInterval(60 * 60)
        case .untilTomorrow:
            let today = calendar.startOfDay(for: now)
            let hour = calendar.component(.hour, from: now)
            let tomorrow = hour < tomorrowHour ? today : (calendar.date(byAdding: .day, value: 1, to: today) ?? now.addingTimeInterval(24 * 3600))
            return calendar.date(bySettingHour: tomorrowHour, minute: 0, second: 0, of: tomorrow) ?? tomorrow.addingTimeInterval(Double(tomorrowHour) * 3600)
        }
    }

    /// The state table. `state` is what the app is in now; for a launch it is `.paused` when a stored
    /// `resumeAt` exists and `.off` otherwise (capture is in memory only and starts off).
    public static func next(
        state: CapturePauseState, resumeAt: Date?, now: Date, event: CapturePauseEvent, calendar: Calendar = .current
    ) -> CapturePauseResult {
        func result(_ s: CapturePauseState, _ at: Date?, _ a: CapturePauseAction) -> CapturePauseResult {
            CapturePauseResult(state: s, resumeAt: at, action: a)
        }
        switch event {
        case .pause(let choice):
            switch state {
            case .on, .paused: return result(.paused, resumeTime(for: choice, now: now, calendar: calendar), .stop)
            case .off: return result(.off, nil, .none)
            }
        case .resumeNow, .turnOn:
            switch state {
            case .paused: return result(.on, nil, .start)
            case .off: return event == .turnOn ? result(.on, nil, .start) : result(.off, nil, .none)
            case .on: return result(.on, nil, .none)
            }
        case .turnOff:
            return result(.off, nil, state == .off ? .none : .stop)
        case .deleteEverything:
            return result(.off, nil, .stop)
        case .timeReached, .appStarted, .wokeFromSleep:
            switch state {
            case .paused:
                // Only a timed pause that is still stored can expire. A missing time is a state that
                // should not exist; it is treated as expired so capture is never stuck off.
                guard let resumeAt, now < resumeAt else { return result(.on, nil, .start) }
                return result(.paused, resumeAt, .none)
            case .off:
                // Expiry never turns on what the person turned off. A leftover time is dropped.
                return result(.off, nil, .none)
            case .on:
                return result(.on, nil, .none)
            }
        }
    }

    /// The state for a launch or a stored value: paused only when a time is stored.
    public static func state(captureOn: Bool, resumeAt: Date?) -> CapturePauseState {
        if captureOn { return .on }
        return resumeAt != nil ? .paused : .off
    }

    /// The clock text after "Paused until": "3:10 PM" for today, "Wed 4:00 AM" for another day.
    public static func untilText(_ at: Date, now: Date, calendar: Calendar = .current, locale: Locale = .current) -> String {
        let time = at.formatted(Date.FormatStyle(date: .omitted, time: .shortened, locale: locale, calendar: calendar, timeZone: calendar.timeZone))
        if calendar.isDate(at, inSameDayAs: now) { return time }
        let day = at.formatted(Date.FormatStyle(locale: locale, calendar: calendar, timeZone: calendar.timeZone).weekday(.abbreviated))
        return "\(day) \(time)"
    }

    /// Seconds until a timer should check again, never negative. The check itself compares the clock.
    public static func secondsUntil(_ resumeAt: Date, now: Date) -> TimeInterval {
        max(0, resumeAt.timeIntervalSince(now))
    }
}
