package dev.taypeer.platform;
import android.os.IBinder;
/** Private lifecycle control, separate from the future document command stream. */
interface IDocumentProcess {
    int initialize(IBinder host);
    oneway void terminate();
}
