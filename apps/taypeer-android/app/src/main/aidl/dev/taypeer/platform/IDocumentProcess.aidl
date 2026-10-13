package dev.taypeer.platform;
import android.os.IBinder;
import android.os.ParcelFileDescriptor;
import dev.taypeer.platform.ICipherPersistence;
import dev.taypeer.platform.ISelectedTransfers;
import dev.taypeer.platform.BiometricInputParcel;
/** Private pipes carry framed document requests; selected plaintext has a separate bounded port. */
interface IDocumentProcess {
    int initialize(IBinder host, in ParcelFileDescriptor commands,
        in ParcelFileDescriptor responses, ICipherPersistence persistence, ISelectedTransfers selected);
    void installBiometricInput(in BiometricInputParcel scope, in ParcelFileDescriptor input);
    oneway void revokeBiometricInput(in BiometricInputParcel scope);
    oneway void terminate();
}
