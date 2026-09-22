package dev.taypeer.platform;
import android.os.IBinder;
/** Debug-only test host. No document or credential commands. */
interface IHostDeathProbe {
    IBinder child();
    oneway void exit();
}
