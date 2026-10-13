package dev.taypeer.platform

import androidx.biometric.BiometricPrompt

/** Minted only by the fixed private OS adapter; never stored in Compose or a public DTO. */
internal interface BiometricPromptRequest {
    val title: String
    val subtitle: String
    val cryptoObject: BiometricPrompt.CryptoObject

    /** OS authorization alone is not an opened document or a durable enrollment ACK. */
    fun authenticated(result: BiometricPrompt.AuthenticationResult)

    /** Synchronously fences the attempt and closes its private key channel before OS cancellation. */
    fun retire(reason: BiometricCancellation)

    /** The OS reported a terminal result; a replacement prompt may now be considered. */
    fun promptDismissed()
}

internal enum class BiometricCancellation {
    CANCEL, MASTER_PASSWORD, BACKGROUND, DATABASE_CHANGED, LOCK,
}
