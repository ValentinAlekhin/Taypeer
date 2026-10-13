package dev.taypeer.platform

import android.content.Context
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import android.system.Os
import android.system.OsConstants
import android.system.ErrnoException
import dev.taypeer.bridge.AndroidException
import dev.taypeer.bridge.CredentialPort
import java.io.File
import java.io.FileOutputStream
import java.nio.ByteBuffer
import java.security.KeyStore
import java.security.MessageDigest
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec
import javax.crypto.AEADBadTagException

/** Host-only credentials. The master password is never passed to this store. */
class KeystoreCredentials internal constructor(
    context: Context,
    private val syncDirectory: (File) -> Unit,
) : CredentialPort {
    constructor(context: Context) : this(context, ::syncCredentialDirectory)
    private val directory = File(context.noBackupFilesDir, "credentials")
    private val keystore = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }

    @Synchronized
    override fun get(service: String, account: String): ByteArray? = guarded {
        val address = address(service, account)
        val file = file(address)
        val size = try { Os.stat(file.path).st_size }
            catch (error: ErrnoException) {
                if (error.errno == OsConstants.ENOENT) return@guarded null
                throw error
            }
        if (size !in 29..(MAX_BYTES + 29).toLong()) throw AndroidException.ProfileInvalid()
        val encoded = ByteArray(size.toInt())
        java.io.DataInputStream(file.inputStream()).use {
            it.readFully(encoded)
            if (it.read() != -1) throw AndroidException.ProfileInvalid()
        }
        if (encoded[0] != 1.toByte()) throw AndroidException.ProfileInvalid()
        val key = keystore.getKey(ALIAS, null) as? SecretKey ?: error("Unavailable key")
        Cipher.getInstance("AES/GCM/NoPadding").run {
            init(Cipher.DECRYPT_MODE, key, GCMParameterSpec(128, encoded.copyOfRange(1, 13)))
            updateAAD(address)
            try { doFinal(encoded, 13, encoded.size - 13) }
            catch (_: AEADBadTagException) { throw AndroidException.ProfileInvalid() }
        }
    }

    @Synchronized
    override fun set(service: String, account: String, bytes: ByteArray) {
        var published = false
        try {
            guarded {
                require(bytes.size <= MAX_BYTES)
                check(directory.isDirectory || directory.mkdirs())
                val address = address(service, account)
                val cipher = Cipher.getInstance("AES/GCM/NoPadding")
                cipher.init(Cipher.ENCRYPT_MODE, key())
                cipher.updateAAD(address)
                val ciphertext = cipher.doFinal(bytes)
                require(cipher.iv.size == 12)
                val temporary = File.createTempFile("credential-", ".pending", directory)
                try {
                    FileOutputStream(temporary).use {
                        it.write(byteArrayOf(1))
                        it.write(cipher.iv)
                        it.write(ciphertext)
                        it.fd.sync()
                    }
                    Os.rename(temporary.path, file(address).path)
                    published = true
                    // Closing and syncing the directory belong to publication ACK.
                    syncDirectory(directory)
                } finally {
                    // Only an unpublished encrypted staging object can remain here.
                    if (temporary.exists()) temporary.delete()
                }
            }
        } catch (error: Exception) {
            if (published) throw AndroidException.CommitUncertain()
            throw error
        } finally { bytes.fill(0) }
    }

    private fun key(): SecretKey {
        (keystore.getKey(ALIAS, null) as? SecretKey)?.let { return it }
        // Never silently replace a lost key for existing credentials.
        check(directory.listFiles()?.none { it.extension == "credential" } == true)
        return KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, "AndroidKeyStore").run {
            init(KeyGenParameterSpec.Builder(ALIAS, KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT)
                .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
                .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
                .setKeySize(256).setRandomizedEncryptionRequired(true).build())
            generateKey()
        }
    }

    private fun address(service: String, account: String): ByteArray {
        val first = service.toByteArray(Charsets.UTF_8)
        val second = account.toByteArray(Charsets.UTF_8)
        require(first.size <= 4096 && second.size <= 4096)
        return ByteBuffer.allocate(8 + first.size + second.size)
            .putInt(first.size).put(first).putInt(second.size).put(second).array()
    }
    private fun file(address: ByteArray): File {
        val digest = MessageDigest.getInstance("SHA-256").digest(address)
        return File(directory, digest.joinToString("") { "%02x".format(it) } + ".credential")
    }
    private inline fun <T> guarded(block: () -> T): T = try { block() }
        catch (error: AndroidException) { throw error }
        catch (_: Exception) { throw AndroidException.Credentials() }

    companion object {
        private const val ALIAS = "dev.taypeer.credentials.v1"
        private const val MAX_BYTES = 65536
    }
}

private fun syncCredentialDirectory(directory: File) {
    val parent = Os.open(directory.path, OsConstants.O_RDONLY, 0)
    try { Os.fsync(parent) } finally { Os.close(parent) }
}
