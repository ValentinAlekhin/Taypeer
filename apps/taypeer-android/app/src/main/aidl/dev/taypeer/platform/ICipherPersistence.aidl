package dev.taypeer.platform;
import android.os.Bundle;
import android.os.ParcelFileDescriptor;
import dev.taypeer.platform.CipherFileResult;
import dev.taypeer.platform.CipherAuthorResult;

/** Host-only ciphertext endpoint. No document values or filesystem paths cross Binder. */
interface ICipherPersistence {
    CipherFileResult createTemporary();
    Bundle snapshot();
    int create(in byte[] controls, in ParcelFileDescriptor[] objects,
        String checkpoint, String baseline);
    Bundle commit(String expected, String control, in byte[] controls,
        in ParcelFileDescriptor[] objects, in String[] remove,
        String checkpoint, String baseline, in byte[] journal);
    CipherAuthorResult author(String authenticated);
    String transportPublic();
    int saveDraft(in ParcelFileDescriptor file);
    CipherFileResult loadDraft();
    int discardDraft();
    Bundle freezeRecovery(String operation, String authenticated);
    Bundle recoverySource(String operation);
    CipherAuthorResult authorRecovery(in byte[] source);
    Bundle recoveryCandidate(in byte[] intent);
    Bundle publishRecovery(in byte[] intent, in byte[] controls,
        in ParcelFileDescriptor[] objects, String checkpoint, String baseline);
    int confirmRecovery(in byte[] publication);
}
