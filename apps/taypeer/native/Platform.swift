import AppKit
import Foundation

// Private helper: no application windows, logs or clipboard contents on stdout.
let application = NSApplication.shared
application.setActivationPolicy(.prohibited)
func event(_ name: String) {
    let data = (name + "\n").data(using: .utf8)!
    FileHandle.standardOutput.write(data)
}
var timedSecretGeneration: Int?
let workspace = NSWorkspace.shared.notificationCenter
let sleepObserver = workspace.addObserver(forName: NSWorkspace.willSleepNotification, object: nil, queue: .main) { _ in event("sleep") }
let sessionObserver = workspace.addObserver(forName: NSWorkspace.sessionDidResignActiveNotification, object: nil, queue: .main) { _ in event("locked") }
let lockObserver = DistributedNotificationCenter.default().addObserver(forName: NSNotification.Name("com.apple.screenIsLocked"), object: nil, queue: .main) { _ in event("locked") }
let activeObserver = workspace.addObserver(forName: NSWorkspace.sessionDidBecomeActiveNotification, object: nil, queue: .main) { _ in event("active") }
let wakeObserver = workspace.addObserver(forName: NSWorkspace.didWakeNotification, object: nil, queue: .main) { _ in event("active") }
let unlockObserver = DistributedNotificationCenter.default().addObserver(forName: NSNotification.Name("com.apple.screenIsUnlocked"), object: nil, queue: .main) { _ in event("active") }
event("ready")
DispatchQueue.global(qos: .userInitiated).async {
    while let line = readLine() {
        guard let data = line.data(using: .utf8),
              let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              let text = object["text"] as? String,
              let secret = object["secret"] as? Bool else { continue }
        let timeout = object["seconds"] as? Double ?? 0
        DispatchQueue.main.async {
            let board = NSPasteboard.general
            board.clearContents()
            var types: [NSPasteboard.PasteboardType] = [.string]
            if secret { types += [NSPasteboard.PasteboardType("org.nspasteboard.ConcealedType"), NSPasteboard.PasteboardType("org.nspasteboard.TransientType")] }
            board.declareTypes(types, owner: nil)
            guard board.setString(text, forType: .string) else { event("clipboard_error"); return }
            let generation = board.changeCount
            timedSecretGeneration = secret && timeout > 0 ? generation : nil
            if secret && timeout > 0 {
                DispatchQueue.main.asyncAfter(deadline: .now() + timeout) {
                    if board.changeCount == generation { board.clearContents() }
                }
            }
        }
    }
    DispatchQueue.main.async {
        let board = NSPasteboard.general
        if let generation = timedSecretGeneration, board.changeCount == generation { board.clearContents() }
        exit(0)
    }
}
RunLoop.main.run()
