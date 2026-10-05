package dev.taypeer.platform;
import android.os.IBinder;
import android.os.ParcelFileDescriptor;
import dev.taypeer.platform.ICipherPersistence;
import dev.taypeer.platform.ISelectedTransfers;
/** Private pipes carry framed document requests; selected plaintext has a separate bounded port. */
interface IDocumentProcess {
    int initialize(IBinder host, in ParcelFileDescriptor commands,
        in ParcelFileDescriptor responses, ICipherPersistence persistence, ISelectedTransfers selected);
    oneway void terminate();
}
