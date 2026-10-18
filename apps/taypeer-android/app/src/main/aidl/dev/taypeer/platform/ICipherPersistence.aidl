package dev.taypeer.platform;
import android.os.Bundle;
import android.os.ParcelFileDescriptor;
import dev.taypeer.platform.CipherFileResult;

/** Host-only ciphertext endpoint. No document values or filesystem paths cross Binder. */
interface ICipherPersistence {
    CipherFileResult createTemporary();
    Bundle snapshot();
    int create(in byte[] controls, in ParcelFileDescriptor[] objects,
        String checkpoint, String baseline);
    Bundle commit(String expected, String control, in byte[] controls,
        in ParcelFileDescriptor[] objects, in String[] remove,
        String checkpoint, String baseline, in byte[] journal);
    Bundle credential(in byte[] scope, boolean prepared);
    int publishCredential(in byte[] scope, in byte[] cipher, boolean prepared);
    int finalizeCredential(in byte[] scope, boolean promotePrepared);
    int activateCredentials(in byte[] capability, String authenticated);
    int activateRecoveryCredentials(in byte[] source, in byte[] capability);
    int saveDraft(in ParcelFileDescriptor file);
    CipherFileResult loadDraft();
    int discardDraft();
    Bundle freezeRecovery(String operation, String authenticated);
    Bundle recoverySource(String operation);
    Bundle recoveryCandidate(in byte[] intent);
    Bundle publishRecovery(in byte[] intent, in byte[] controls,
        in ParcelFileDescriptor[] objects, String checkpoint, String baseline);
    Bundle publishRecoveryCredentials(in byte[] intent, in byte[] controls,
        in ParcelFileDescriptor[] objects, String checkpoint, String baseline,
        in byte[] cipher, in byte[] capability);
    int confirmRecovery(in byte[] publication);
}
