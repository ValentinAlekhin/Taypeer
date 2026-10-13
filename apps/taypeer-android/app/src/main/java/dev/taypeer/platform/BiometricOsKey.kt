package dev.taypeer.platform

import android.content.Context
import android.os.Build
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyInfo
import android.security.keystore.KeyProperties
import androidx.biometric.BiometricManager
import java.security.KeyFactory
import java.security.KeyPairGenerator
import java.security.KeyStore
import java.security.PrivateKey
import java.security.interfaces.RSAPublicKey
import java.security.spec.MGF1ParameterSpec
import java.security.spec.RSAKeyGenParameterSpec
import java.util.UUID
import javax.crypto.Cipher
import javax.crypto.spec.OAEPParameterSpec
import javax.crypto.spec.PSource

/** Private OS adapter's verified key handle. UI and worker bindings never choose an alias. */
internal class BiometricOsKey private constructor(
    private val alias: String,
    private val public: RSAPublicKey,
    private val private: PrivateKey,
) {
    /** Public RSA material only; the scoped worker capability is checked separately. */
    fun publicSpki(): ByteArray = public.encoded.copyOf()
    internal fun registryAlias(): String = alias

    /** Every attempt gets a new Cipher and must use its exact CryptoObject in the OS prompt. */
    fun decryptCipher(): Cipher = Cipher.getInstance("RSA/ECB/OAEPPadding").apply {
        init(Cipher.DECRYPT_MODE, private,
            OAEPParameterSpec("SHA-256", "MGF1", MGF1ParameterSpec.SHA1, PSource.PSpecified.DEFAULT))
    }

    fun delete() { keystore().deleteEntry(alias) }

    companion object {
        private const val PREFIX = "taypeer_biometry_v1_"
        private fun keystore(): KeyStore = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }

        /** The digest comes from the worker's public scope, never a human name or external file. */
        fun create(context: Context, scopeDigest: String): BiometricOsKey {
            require(scopeDigest.matches(Regex("[0-9a-f]{64}")))
            check(BiometricManager.from(context).canAuthenticate(BiometricManager.Authenticators.BIOMETRIC_STRONG) == BiometricManager.BIOMETRIC_SUCCESS) {
                "Strong biometric authentication unavailable"
            }
            val alias = PREFIX + scopeDigest + "_" + UUID.randomUUID().toString().replace("-", "")
            val builder = KeyGenParameterSpec.Builder(alias, KeyProperties.PURPOSE_DECRYPT)
                .setAlgorithmParameterSpec(RSAKeyGenParameterSpec(2048, RSAKeyGenParameterSpec.F4))
                .setDigests(KeyProperties.DIGEST_SHA256)
                .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_RSA_OAEP)
                .setUserAuthenticationRequired(true)
                .setUserAuthenticationParameters(0, KeyProperties.AUTH_BIOMETRIC_STRONG)
                .setInvalidatedByBiometricEnrollment(true)
            if (Build.VERSION.SDK_INT >= 35) builder.setMgf1Digests(KeyProperties.DIGEST_SHA1)
            try {
                KeyPairGenerator.getInstance(KeyProperties.KEY_ALGORITHM_RSA, "AndroidKeyStore").apply {
                    initialize(builder.build())
                }.generateKeyPair()
                return verified(alias)
            } catch (failure: Exception) {
                try { keystore().deleteEntry(alias) } catch (_: Exception) { /* No active registry was published. */ }
                throw failure
            }
        }

        /** Called only by the private registry after its exact scope and public SPKI match. */
        internal fun registered(alias: String, scopeDigest: String, expectedSpki: ByteArray): BiometricOsKey {
            require(scopeDigest.matches(Regex("[0-9a-f]{64}")))
            require(alias.matches(Regex(PREFIX + scopeDigest + "_[0-9a-f]{32}")))
            val verified = verified(alias)
            check(verified.publicSpki().contentEquals(expectedSpki)) { "Biometric public binding changed" }
            return verified
        }

        /** Native stores the digest of public DER, never a provider/private-key object. */
        internal fun registeredDigest(alias: String, scopeDigest: String, expectedDigest: String): BiometricOsKey {
            require(scopeDigest.matches(Regex("[0-9a-f]{64}")))
            require(expectedDigest.matches(Regex("[0-9a-f]{64}")))
            require(alias.matches(Regex(PREFIX + scopeDigest + "_[0-9a-f]{32}")))
            val verified = verified(alias)
            val actual = java.security.MessageDigest.getInstance("SHA-256").digest(verified.publicSpki())
                .joinToString("") { "%02x".format(it.toInt() and 255) }
            check(actual == expectedDigest) { "Biometric public binding changed" }
            return verified
        }

        /** An invalidated key can still be deleted, but only at its exact Native-owned alias. */
        internal fun deleteRegistered(alias: String, scopeDigest: String, generation: String) {
            require(scopeDigest.matches(Regex("[0-9a-f]{64}")))
            require(generation.matches(Regex("[0-9a-f]{32}")))
            require(alias == PREFIX + scopeDigest + "_" + generation)
            keystore().deleteEntry(alias)
        }

        private fun verified(alias: String): BiometricOsKey {
            val store = keystore()
            val private = store.getKey(alias, null) as? PrivateKey ?: error("Biometric key unavailable")
            val public = store.getCertificate(alias)?.publicKey as? RSAPublicKey ?: error("Biometric public key unavailable")
            val info = KeyFactory.getInstance("RSA", "AndroidKeyStore").getKeySpec(private, KeyInfo::class.java)
            check(public.modulus.bitLength() == 2048 && public.publicExponent == RSAKeyGenParameterSpec.F4)
            check(info.keySize == 2048 && info.purposes == KeyProperties.PURPOSE_DECRYPT)
            check(info.digests.contentEquals(arrayOf(KeyProperties.DIGEST_SHA256)))
            check(info.encryptionPaddings.contentEquals(arrayOf(KeyProperties.ENCRYPTION_PADDING_RSA_OAEP)))
            // Keystore2 on Android 12 reports per-use as 0; legacy KeyInfo documents -1.
            // Both require the exact CryptoObject operation; positive time windows are refused.
            val duration = info.userAuthenticationValidityDurationSeconds
            check(info.isUserAuthenticationRequired && (duration == 0 || duration == -1)) {
                "Biometric authentication policy mismatch: required=${info.isUserAuthenticationRequired}, duration=${info.userAuthenticationValidityDurationSeconds}"
            }
            check(info.userAuthenticationType == KeyProperties.AUTH_BIOMETRIC_STRONG && info.isInvalidatedByBiometricEnrollment) {
                "Biometric authenticator policy mismatch: type=${info.userAuthenticationType}, enrollmentInvalidation=${info.isInvalidatedByBiometricEnrollment}"
            }
            return BiometricOsKey(alias, public, private)
        }
    }
}
