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
class DesktopInteropTest {
    @Test fun actualDesktopTorInterop() {
        val desktopCode = InstrumentationRegistry.getArguments().getString("desktopCode")
        assumeTrue(!desktopCode.isNullOrEmpty())
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        val e = Envelopes(context, testing = true); e.wipe()
        e.setup("android interop normal".toByteArray(), "android interop cleanup".toByteArray())
        val cache=InstrumentationRegistry.getArguments().getString("cacheDir")
        if(cache!=null){val dir=File(e.root,"tor").apply{mkdirs()};File(cache).listFiles().orEmpty().filter{it.isFile&&it.name.startsWith("cached-")}.forEach{it.copyTo(File(dir,it.name),overwrite=true)}}
        val key = e.unlock("android interop normal".toByteArray()).key!!
        fun call(req: JSONObject): JSONObject {
            Native.nativeHeartbeat()
            val v = JSONObject(Native.nativeCommand(req.toString())); check(!v.optBoolean("error"))
            Native.nativeCommand(JSONObject().put("op", "displayed").put("epoch", v.optLong("epoch")).toString())
            return v
        }
        fun waitFor(cid: String = "", predicate: (JSONObject) -> Boolean): JSONObject {
            val end = System.currentTimeMillis() + 900000
            var report = System.currentTimeMillis()
            while (System.currentTimeMillis() < end) {
                val v = call(JSONObject().put("op", "view").put("conversation", cid))
                if (predicate(v)) return v
                if(System.currentTimeMillis()-report>20000){File(e.root,"interop.status").writeText("network=${v.optString("network")} messages=${v.getJSONObject("view").optJSONObject("active")?.optJSONArray("messages")?.length() ?: 0} group=${v.getJSONObject("view").optJSONObject("active")?.optJSONObject("group")?.optInt("ready") ?: -1} joined=${v.getJSONObject("view").optJSONObject("active")?.optJSONObject("group")?.optInt("joined") ?: -1}");report=System.currentTimeMillis()}
                Thread.sleep(100)
            }
            error("Android desktop Tor stage timed out")
        }
        val binary = File(context.applicationInfo.nativeLibraryDir, "libtor.so").absolutePath
        try {
            assertTrue(Native.nativeOpen(e.root.absolutePath, binary, key, false))
            val own = waitFor { !it.getJSONObject("view").isNull("own_code") }.getJSONObject("view").getString("own_code")
            File(e.root, "android.code").writeText(own)
            val cid = call(JSONObject().put("op", "pair").put("code", desktopCode)).getJSONObject("view").getJSONObject("active").getString("id")
            call(JSONObject().put("op", "send").put("conversation", cid).put("text", "Android to desktop"))
            waitFor(cid) { val m=it.getJSONObject("view").getJSONObject("active").getJSONArray("messages"); m.length()==2 && (0 until m.length()).all { i -> m.getJSONObject(i).getBoolean("delivered") } }
            waitFor(cid) { File(e.root, "desktop.exchange").isFile }
            call(JSONObject().put("op", "delete").put("conversation", cid))
            waitFor(cid) { val chat=it.getJSONObject("view").getJSONObject("active"); chat.optBoolean("deletion") && chat.getJSONArray("messages").length()==0 }
            waitFor(cid) { File(e.root, "desktop.confirmed").isFile }
            if(InstrumentationRegistry.getArguments().getString("groupTest")=="1") {
                waitFor { File(e.root,"group.invite").isFile }
                val invite=File(e.root,"group.invite").readText()
                call(JSONObject().put("op","preview_group").put("code",invite))
                val gid=call(JSONObject().put("op","import_group").put("code",invite)).getJSONObject("view").getJSONObject("active").getString("id")
                waitFor(gid){it.getJSONObject("view").getJSONObject("active").getJSONObject("group").getInt("ready")==3}
                call(JSONObject().put("op","send").put("conversation",gid).put("text","Android group author"))
                waitFor(gid){it.getJSONObject("view").getJSONObject("active").getJSONArray("messages").length()==3 && File(e.root,"group.exchange").isFile}
                call(JSONObject().put("op","delete").put("conversation",gid))
                waitFor(gid){val chat=it.getJSONObject("view").getJSONObject("active"); chat.getJSONArray("messages").length()==0&&chat.optBoolean("deletion")&&File(e.root,"group.confirmed").isFile}
                Native.nativeRevoke();Native.nativeClose();File(e.root,"android.offline").writeText("ok")
                val end=System.currentTimeMillis()+900000
                while(!File(e.root,"group.reopen").isFile){check(System.currentTimeMillis()<end);Thread.sleep(100)}
                assertTrue(Native.nativeOpen(e.root.absolutePath,binary,key,false))
                waitFor(gid){val messages=it.getJSONObject("view").getJSONObject("active").getJSONArray("messages");messages.length()==1&&messages.getJSONObject(0).getString("text")=="other author during offline"&&File(e.root,"group.final.confirmed").isFile}
            }
            Native.nativeRevoke(); Native.nativeClose()
            assertTrue(Native.nativeOpen(e.root.absolutePath, binary, key, false))
            assertEquals(own, waitFor { !it.getJSONObject("view").isNull("own_code") }.getJSONObject("view").getString("own_code"))
        } finally { key.fill(0); Native.nativeRevoke(); Native.nativeClose(); e.wipe() }
    }
}
