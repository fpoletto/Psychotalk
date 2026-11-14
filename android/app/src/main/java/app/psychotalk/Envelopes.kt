package app.psychotalk

import android.content.Context
import android.os.Build
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyInfo
import android.security.keystore.KeyProperties
import android.system.Os
import android.system.OsConstants
import android.util.AtomicFile
import java.io.File
import java.security.KeyStore
import java.security.SecureRandom
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.SecretKeyFactory
import javax.crypto.spec.GCMParameterSpec
import javax.crypto.spec.SecretKeySpec

class Envelopes(context: Context, testing: Boolean = false) {
    val root = File(context.noBackupFilesDir, if (testing) "psychotalk-tests" else "psychotalk").apply { mkdirs() }
    private val alias = if (testing) "psychotalk.v1.tests" else "psychotalk.v1.vault"
    private val random = SecureRandom()
    private fun store() = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }
    private fun key() = store().getKey(alias, null) as SecretKey
    val configured: Boolean get() = File(root, "open.env").isFile && File(root, "clean.env").isFile && File(root, "vault.db").isFile
    fun syncDirectory() {
        val fd = Os.open(root.absolutePath, OsConstants.O_RDONLY, 0)
        try { Os.fsync(fd) } finally { Os.close(fd) }
    }
    private fun atomic(name: String, bytes: ByteArray) {
        val file = AtomicFile(File(root, name))
        val stream = file.startWrite()
        try { stream.write(bytes); file.finishWrite(stream); syncDirectory() }
        catch (e: Exception) { file.failWrite(stream); throw e }
    }
    fun recover() {
        if (File(root, "wipe.intent").exists() || File(root, "setup.intent").exists()) wipe()
        check(configured || root.listFiles().orEmpty().isEmpty()) { "Armazenamento incompleto; mantenha o aplicativo bloqueado." }
    }
    fun setup(normal: ByteArray, cleanup: ByteArray) {
        check(!configured && !normal.contentEquals(cleanup) && normal.isNotEmpty() && cleanup.isNotEmpty())
        atomic("setup.intent", byteArrayOf(1))
        val generator = KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, "AndroidKeyStore")
        generator.init(KeyGenParameterSpec.Builder(alias, KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT)
            .setBlockModes(KeyProperties.BLOCK_MODE_GCM).setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
            .setKeySize(256).setRandomizedEncryptionRequired(true).build())
        generator.generateKey()
        val dek = ByteArray(32).also(random::nextBytes)
        val token = ByteArray(32).also(random::nextBytes)
        try {
            seal("open.env", normal, byteArrayOf(1) + dek)
            seal("clean.env", cleanup, byteArrayOf(2) + token)
            check(Native.nativeInitialize(root.absolutePath, dek))
            check(File(root, "setup.intent").delete()); syncDirectory()
        } finally { dek.fill(0); token.fill(0) }
    }
    private fun seal(name: String, password: ByteArray, payload: ByteArray) {
        val salt = ByteArray(16).also(random::nextBytes)
        val header = byteArrayOf(1) + salt
        val derived = Native.nativeDerive(password, salt)
        check(derived.size == 32)
        try {
            val inner = Cipher.getInstance("AES/GCM/NoPadding")
            inner.init(Cipher.ENCRYPT_MODE, SecretKeySpec(derived, "AES")); inner.updateAAD(header)
            val encrypted = inner.iv + inner.doFinal(payload)
            try {
                val outer = Cipher.getInstance("AES/GCM/NoPadding")
                outer.init(Cipher.ENCRYPT_MODE, key()); outer.updateAAD(header)
                atomic(name, header + outer.iv + outer.doFinal(encrypted))
            } finally { encrypted.fill(0) }
        } finally { derived.fill(0); payload.fill(0) }
    }
    private fun unseal(name: String, password: ByteArray, kind: Byte): ByteArray? {
        val bytes = runCatching { File(root, name).readBytes() }.getOrDefault(ByteArray(106))
        val salt = if (bytes.size == 106) bytes.copyOfRange(1, 17) else ByteArray(16)
        // Derive for both envelopes even when authentication or the file format fails.
        val derived = Native.nativeDerive(password, salt)
        try {
            check(bytes.size == 106 && bytes[0] == 1.toByte() && derived.size == 32)
            val header = bytes.copyOfRange(0, 17)
            val outer = Cipher.getInstance("AES/GCM/NoPadding")
            outer.init(Cipher.DECRYPT_MODE, key(), GCMParameterSpec(128, bytes.copyOfRange(17, 29)))
            outer.updateAAD(header)
            val innerBytes = outer.doFinal(bytes.copyOfRange(29, bytes.size))
            try {
                val inner = Cipher.getInstance("AES/GCM/NoPadding")
                inner.init(Cipher.DECRYPT_MODE, SecretKeySpec(derived, "AES"), GCMParameterSpec(128, innerBytes.copyOfRange(0, 12)))
                inner.updateAAD(header)
                val payload = inner.doFinal(innerBytes.copyOfRange(12, innerBytes.size))
                try { check(payload.size == 33 && payload[0] == kind); return payload.copyOfRange(1, 33) }
                finally { payload.fill(0) }
            } finally { innerBytes.fill(0) }
        } catch (_: Exception) { return null }
        finally { derived.fill(0); salt.fill(0); bytes.fill(0) }
    }
    data class Unlock(val key: ByteArray?, val cleanup: Boolean)
    fun unlock(password: ByteArray): Unlock {
        val normal = unseal("open.env", password, 1)
        val cleanup = unseal("clean.env", password, 2)
        return if (cleanup != null) { normal?.fill(0); cleanup.fill(0); Unlock(null, true) }
        else Unlock(normal, false)
    }
    fun wipe() {
        Native.nativeRevoke()
        atomic("wipe.intent", byteArrayOf(1))
        val ks = store(); ks.deleteEntry(alias); check(!ks.containsAlias(alias))
        Native.nativeClose()
        for (file in root.listFiles().orEmpty()) if (file.name != "wipe.intent") check(file.deleteRecursively())
        syncDirectory(); check(File(root, "wipe.intent").delete()); syncDirectory()
    }
    fun securityLevel(): String {
        val info = SecretKeyFactory.getInstance(key().algorithm, "AndroidKeyStore").getKeySpec(key(), KeyInfo::class.java) as KeyInfo
        return if (Build.VERSION.SDK_INT >= 31) when (info.securityLevel) {
            KeyProperties.SECURITY_LEVEL_STRONGBOX -> "Keystore: StrongBox"
            KeyProperties.SECURITY_LEVEL_TRUSTED_ENVIRONMENT -> "Keystore: hardware (TEE)"
            else -> "Keystore: somente software"
        } else if (info.isInsideSecureHardware) "Keystore: hardware" else "Keystore: somente software"
    }
}
