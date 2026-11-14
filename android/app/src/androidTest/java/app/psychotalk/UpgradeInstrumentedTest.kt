package app.psychotalk

import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import org.junit.Assert.*
import org.junit.Assume.assumeTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.json.JSONObject
import java.io.File

@RunWith(AndroidJUnit4::class)
class UpgradeInstrumentedTest {
    private val context get() = InstrumentationRegistry.getInstrumentation().targetContext
    private fun mode(expected: String) { assumeTrue(InstrumentationRegistry.getArguments().getString("legacyMode") in if (expected == "seed") listOf("seed", "seed2", "seed3") else listOf("verify", "verify3")) }
    private fun view() = JSONObject(Native.nativeCommand("{\"op\":\"view\"}"))
    private fun code(): String {
        val end = System.currentTimeMillis() + 20000
        while (System.currentTimeMillis() < end) {
            Native.nativeHeartbeat()
            val v = view().optJSONObject("view")
            if (v != null && !v.isNull("own_code")) return v.getString("own_code")
            Thread.sleep(100)
        }
        error("test onion not initialized")
    }
    @Test fun seedOldApk() {
        mode("seed")
        val expectedVersion = when (InstrumentationRegistry.getArguments().getString("legacyMode")) { "seed3" -> "0.3.0"; "seed2" -> "0.2.0"; else -> "0.1.2" }
        assertTrue(context.packageManager.getPackageInfo(context.packageName, 0).versionName!!.startsWith(expectedVersion))
        val e = Envelopes(context, testing = true); e.wipe()
        e.setup("migration normal".toByteArray(), "migration cleanup".toByteArray())
        val key = e.unlock("migration normal".toByteArray()).key!!
        try {
            assertTrue(Native.nativeOpen(e.root.absolutePath, File(context.applicationInfo.nativeLibraryDir, "libtor.so").absolutePath, key, false))
            File(e.root, "migration.expected").writeText(code())
            val peer = "psychotalk:1:fivcukrkfivcukrkfivcukrkfivcukrkfivcukrkfivcukrkfivjcrid.onion:${"2a".repeat(32)}"
            fun call(req: JSONObject) { Native.nativeHeartbeat(); assertFalse(JSONObject(Native.nativeCommand(req.toString())).optBoolean("error")) }
            call(JSONObject().put("op", "pair").put("code", peer))
            call(JSONObject().put("op", "send").put("text", "before delete"))
            call(JSONObject().put("op", "delete"))
            call(JSONObject().put("op", "send").put("text", "after delete"))
        } finally { key.fill(0); Native.nativeRevoke(); Native.nativeClose() }
    }
    @Test fun verifyUpgradeKeepsVaultAndKeystore() {
        mode("verify")
        val e = Envelopes(context, testing = true)
        assertTrue(e.configured)
        val expected = File(e.root, "migration.expected").readText()
        val key = e.unlock("migration normal".toByteArray()).key!!
        try {
            assertTrue(Native.nativeOpen(e.root.absolutePath, File(context.applicationInfo.nativeLibraryDir, "libtor.so").absolutePath, key, false))
            assertEquals(expected, code())
            val conversations = view().getJSONObject("view").getJSONArray("conversations")
            assertEquals(1, conversations.length())
            val cid = conversations.getJSONObject(0).getString("id")
            val active = JSONObject(Native.nativeCommand(JSONObject().put("op", "view").put("conversation", cid).toString())).getJSONObject("view").getJSONObject("active")
            assertEquals(1, active.getJSONArray("messages").length())
            assertEquals("after delete", active.getJSONArray("messages").getJSONObject(0).getString("text"))
            assertFalse(active.getJSONArray("messages").getJSONObject(0).getBoolean("delivered"))
            assertFalse(active.getBoolean("deletion"))
            assertFalse(JSONObject(Native.nativeCommand(JSONObject().put("op", "rename").put("conversation", cid).put("tagname", "@preservado").toString())).optBoolean("error"))
        } finally { key.fill(0); Native.nativeRevoke(); Native.nativeClose(); e.wipe() }
    }
}
