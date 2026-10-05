package dev.taypeer.platform;
import android.os.IBinder;
/** Debug-only lifecycle smoke; document acceptance uses the real DocumentService. */
interface ILifecycleProbe {
    int initialize(IBinder host);
    oneway void terminate();
}
