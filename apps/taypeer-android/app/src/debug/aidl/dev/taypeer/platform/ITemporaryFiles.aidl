package dev.taypeer.platform;
import android.os.ParcelFileDescriptor;
/** Ciphertext-only allocation capability, without directory access. */
interface ITemporaryFiles {
    ParcelFileDescriptor create();
}
