package app.psychotalk

import androidx.compose.runtime.MutableState
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.v2.createAndroidComposeRule
import androidx.test.platform.app.InstrumentationRegistry
import org.junit.Assert.*
import org.junit.Rule
import org.junit.Test
import org.json.JSONObject
import java.io.File

class ConversationUiTest {
    @get:Rule val compose = createAndroidComposeRule<MainActivity>()
    @Test fun realVaultListNavigationTagnameAndScopedSend() {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        val e = Envelopes(context, testing = true); e.wipe()
        e.setup("ui normal".toByteArray(), "ui cleanup".toByteArray())
        val key = e.unlock("ui normal".toByteArray()).key!!
        fun call(req: JSONObject): JSONObject {
            val result = JSONObject(Native.nativeCommand(req.toString()))
            assertFalse(result.optBoolean("error")); return result
        }
        try {
            compose.waitForIdle()
            assertTrue(Native.nativeOpen(e.root.absolutePath, File(context.applicationInfo.nativeLibraryDir, "libtor.so").absolutePath, key, false))
            Native.nativeHeartbeat()
            val onion = "fivcukrkfivcukrkfivcukrkfivcukrkfivcukrkfivcukrkfivjcrid.onion"
            call(JSONObject().put("op", "pair").put("tagname", "@bia").put("code", "psychotalk:1:$onion:${"2a".repeat(32)}"))
            call(JSONObject().put("op", "pair").put("tagname", "@caio").put("code", "psychotalk:1:fmvswkzlfmvswkzlfmvswkzlfmvswkzlfmvswkzlfmvswkzlfmvuqiad.onion:${"2b".repeat(32)}"))
            val snapshot = call(JSONObject().put("op", "view"))
            compose.runOnIdle {
                val opened = MainActivity::class.java.getDeclaredField("opened").apply { isAccessible = true }
                opened.setBoolean(compose.activity, true)
                val field = MainActivity::class.java.getDeclaredField("screen\$delegate").apply { isAccessible = true }
                @Suppress("UNCHECKED_CAST")
                val state = field.get(compose.activity) as MutableState<Screen>
                state.value = Screen(stage = Stage.ACTIVE, ready = true, data = snapshot, level = e.securityLevel())
            }
            compose.onNodeWithText("Conversas").assertExists()
            compose.onNodeWithText("@bia").performClick()
            compose.onNodeWithText("Opções").performClick()
            compose.onNodeWithText("Editar tagname").performClick()
            compose.onNode(hasSetTextAction() and hasAnyAncestor(isDialog())).performTextReplacement("@melhor_amiga")
            compose.onNodeWithText("Salvar tagname").performClick()
            compose.waitUntil(5000) { compose.onAllNodesWithText("@melhor_amiga").fetchSemanticsNodes().isNotEmpty() }
            compose.onNodeWithText("← Conversas").performClick()
            compose.onNodeWithText("@caio").performClick()
            compose.onNodeWithText("Mensagem de texto").performClick()
            compose.onNodeWithText("Mensagem de texto").performTextInput("somente para caio")
            compose.onNodeWithText("Enviar").assertIsDisplayed().performClick()
            compose.waitUntil(5000) { compose.onAllNodesWithText("somente para caio").fetchSemanticsNodes().isNotEmpty() }
            compose.onNodeWithText("← Conversas").performClick()
            compose.onNodeWithText("@melhor_amiga").performClick()
            compose.onNodeWithText("somente para caio").assertDoesNotExist()
            compose.onNodeWithText("@melhor_amiga").assertExists()
        } finally { key.fill(0); Native.nativeRevoke(); Native.nativeClose(); e.wipe() }
    }
}
