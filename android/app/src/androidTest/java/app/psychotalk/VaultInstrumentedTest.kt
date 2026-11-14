package app.psychotalk

import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith
import java.io.File
import java.security.KeyStore
import org.json.JSONObject

@RunWith(AndroidJUnit4::class)
class VaultInstrumentedTest {
    private val context get() = InstrumentationRegistry.getInstrumentation().targetContext
    private fun fixture(test: (Envelopes) -> Unit) {
        val e = Envelopes(context, testing = true)
        e.wipe()
        try { test(e) } finally { e.wipe() }
    }
    @Test fun actualJniSqlcipherTwoPasswordsAndWrongPassword() = fixture { e ->
        e.setup("normal test password".toByteArray(), "cleanup test password".toByteArray())
        assertTrue(e.configured)
        val wrong = e.unlock("incorrect password".toByteArray())
        assertNull(wrong.key); assertFalse(wrong.cleanup); assertTrue(e.configured)
        val normal = e.unlock("normal test password".toByteArray())
        assertNotNull(normal.key); assertFalse(normal.cleanup); normal.key!!.fill(0)
        assertTrue(e.securityLevel().startsWith("Keystore:"))
        val bytes = File(e.root, "vault.db").readBytes()
        assertFalse(bytes.copyOfRange(0, 16).contentEquals("SQLite format 3\u0000".toByteArray()))
    }
    @Test fun cleanupWorksWithoutDatabaseAndNeverReturnsDataKey() = fixture { e ->
        e.setup("normal".toByteArray(), "cleanup".toByteArray())
        assertTrue(File(e.root, "vault.db").delete())
        val clean = e.unlock("cleanup".toByteArray())
        assertTrue(clean.cleanup); assertNull(clean.key)
        e.wipe(); assertFalse(e.configured); assertTrue(e.root.listFiles().orEmpty().isEmpty())
    }
    @Test fun durableCleanupIntentResumesBeforeAnySession() = fixture { e ->
        e.setup("normal".toByteArray(), "cleanup".toByteArray())
        File(e.root, "wipe.intent").outputStream().use { it.write(1); it.fd.sync() }
        e.recover(); assertFalse(e.configured); assertTrue(e.root.listFiles().orEmpty().isEmpty())
    }
    @Test fun keystoreIsRequiredInAdditionToPassword() = fixture { e ->
        e.setup("normal".toByteArray(), "cleanup".toByteArray())
        KeyStore.getInstance("AndroidKeyStore").apply { load(null); deleteEntry("psychotalk.v1.tests") }
        assertNull(e.unlock("normal".toByteArray()).key); assertTrue(e.configured)
    }
    @Test fun guardianTorExecutableRunsOnAndroid() {
        val process = ProcessBuilder(File(context.applicationInfo.nativeLibraryDir, "libtor.so").absolutePath, "--version").start()
        val output = process.inputStream.bufferedReader().readText()
        assertEquals(0, process.waitFor()); assertTrue(output.contains("0.4.9.14"))
    }
    @Test fun realAndroidSessionRestoresOnionAndExpiresWithoutForegroundHeartbeat() = fixture { e ->
        e.setup("normal".toByteArray(), "cleanup".toByteArray())
        val key = e.unlock("normal".toByteArray()).key!!
        val binary = File(context.applicationInfo.nativeLibraryDir, "libtor.so").absolutePath
        fun code(): String {
            val deadline = System.currentTimeMillis() + 20000
            while (System.currentTimeMillis() < deadline) {
                Native.nativeHeartbeat()
                val data = JSONObject(Native.nativeCommand("{\"op\":\"view\"}"))
                val view = data.optJSONObject("view")
                if (view != null && !view.isNull("own_code")) return view.getString("own_code")
                Thread.sleep(100)
            }
            error("onion service not initialized")
        }
        try {
            assertTrue(Native.nativeOpen(e.root.absolutePath, binary, key, false))
            val original = code()
            Native.nativeRevoke(); Native.nativeClose()
            assertTrue(Native.nativeOpen(e.root.absolutePath, binary, key, false))
            assertEquals(original, code())
            assertFalse(e.root.walkTopDown().any { it.name.contains("hs_ed25519_secret_key") })
            Thread.sleep(2600)
            assertTrue(JSONObject(Native.nativeCommand("{\"op\":\"view\"}")).optBoolean("error"))
        } finally { key.fill(0); Native.nativeRevoke(); Native.nativeClose() }
    }
}
