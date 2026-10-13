package dev.taypeer

import android.os.BadParcelableException
import android.os.Parcel
import android.os.ParcelFileDescriptor
import androidx.test.platform.app.InstrumentationRegistry
import dev.taypeer.bridge.*
import dev.taypeer.platform.*
import java.io.File
import java.util.UUID
import org.junit.Assert.*
import org.junit.Test

/** Real private Binder refusal and typed parcel bounds, without enrolled OS credentials. */
class BiometricInputPortTest {
    private fun scope(purpose: BiometricPurpose = BiometricPurpose.ENROLLMENT_PROOF) = BiometricInputSpec(
        "10".repeat(32), "20".repeat(32), "PUBLIC database identity", "30".repeat(32), ULong.MAX_VALUE,
        "40".repeat(32), "50".repeat(32), "60".repeat(32), ULong.MAX_VALUE,
        "70".repeat(32), "80".repeat(32), purpose)

    @Test fun exactParcelRetainsBothPurposesAndUnsignedGenerationWithoutOrdinalPolicy() {
        BiometricPurpose.entries.forEach { purpose ->
            val parcel = Parcel.obtain()
            try {
                val input = scope(purpose)
                BiometricInputParcel(input).writeToParcel(parcel, 0)
                parcel.setDataPosition(0)
                assertEquals(input, BiometricInputParcel.CREATOR.createFromParcel(parcel).scope)
                assertEquals(parcel.dataSize(), parcel.dataPosition())
            } finally { parcel.recycle() }
        }
    }

    @Test fun malformedDigestUtf8DatabaseAndZeroGenerationNeverReachBinder() {
        val valid = scope()
        assertThrows(BadParcelableException::class.java) { BiometricInputParcel(valid.copy(attempt = "A".repeat(64))) }
        assertThrows(BadParcelableException::class.java) { BiometricInputParcel(valid.copy(database = "я".repeat(65))) }
        assertThrows(BadParcelableException::class.java) { BiometricInputParcel(valid.copy(generation = 0uL)) }
        assertThrows(BadParcelableException::class.java) { BiometricInputParcel(valid.copy(input = "10")) }
    }

    @Test fun unarmedPrivateInputAndReplayedRevocationCannotAcknowledgeOrMutateARealOpenWorker() {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        val directory = File(context.noBackupFilesDir, "PUBLIC-biometric-input-${UUID.randomUUID()}")
        try {
            Host(directory.path, KeystoreCredentials(context)).use { host ->
                DocumentManager(context, host).use { manager ->
                    val session = manager.create("PUBLIC unarmed biometric port", null, newOperation(), "PUBLIC biometric port master")
                    val path = manager.catalog().single().path
                    val before = File(path).readBytes()
                    val process = manager.process(session)
                    val pipe = ParcelFileDescriptor.createPipe()
                    try {
                        val input = scope().copy(database = session.databaseId(), generation = session.generation())
                        assertThrows(AndroidException.Biometry::class.java) { process.installBiometricInput(input, pipe[0].fd) }
                        // A denied install retains the caller's original descriptor.
                        assertTrue(pipe[0].fileDescriptor.valid())
                        process.revokeBiometricInput(input)
                        process.revokeBiometricInput(input)
                        assertEquals(SessionAccess.OPEN, session.access())
                        assertEquals("PUBLIC unarmed biometric port", session.overview("").name)
                        assertArrayEquals(before, File(path).readBytes())
                    } finally { pipe.forEach { it.close() } }
                }
            }
        } finally { assertTrue(directory.deleteRecursively()) }
    }
}
