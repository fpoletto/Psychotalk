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

class GroupUiTest {
    @get:Rule val compose = createAndroidComposeRule<MainActivity>()
    @Test fun realVaultCreatePreviewAcceptRenameAndRecipientDeletionProgress() {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        val e = Envelopes(context, testing = true); e.wipe()
        e.setup("group normal".toByteArray(), "group cleanup".toByteArray())
        val key = e.unlock("group normal".toByteArray()).key!!
        fun call(req: JSONObject): JSONObject {
            Native.nativeHeartbeat()
            val result = JSONObject(Native.nativeCommand(req.toString()))
            assertFalse(result.optBoolean("error")); return result
        }
        try {
            compose.waitForIdle()
            assertTrue(Native.nativeOpen(e.root.absolutePath, File(context.applicationInfo.nativeLibraryDir, "libtor.so").absolutePath, key, false))
            val end = System.currentTimeMillis() + 20000
            while (call(JSONObject().put("op", "view")).getJSONObject("view").isNull("own_code")) { check(System.currentTimeMillis() < end); Thread.sleep(100) }
            call(JSONObject().put("op", "pair").put("tagname", "@bia").put("code", "psychotalk:1:fivcukrkfivcukrkfivcukrkfivcukrkfivcukrkfivcukrkfivjcrid.onion:${"2a".repeat(32)}"))
            call(JSONObject().put("op", "pair").put("tagname", "@caio").put("code", "psychotalk:1:fmvswkzlfmvswkzlfmvswkzlfmvswkzlfmvswkzlfmvswkzlfmvuqiad.onion:${"2b".repeat(32)}"))
            val snapshot = call(JSONObject().put("op", "view"))
            compose.runOnIdle {
                MainActivity::class.java.getDeclaredField("opened").apply { isAccessible = true }.setBoolean(compose.activity, true)
                val field = MainActivity::class.java.getDeclaredField("screen\$delegate").apply { isAccessible = true }
                @Suppress("UNCHECKED_CAST") val state = field.get(compose.activity) as MutableState<Screen>
                state.value = Screen(stage = Stage.ACTIVE, ready = true, data = snapshot, level = e.securityLevel())
            }
            compose.onNodeWithText("Novo grupo").performClick()
            compose.onNodeWithText("Nome fixo do grupo").performTextInput("Equipe UI")
            compose.onAllNodes(isToggleable())[0].performScrollTo().performClick().assertIsOn()
            compose.onNodeWithText("Criar grupo fixo").assertIsNotEnabled()
            compose.onAllNodes(isToggleable())[1].performScrollTo().performClick().assertIsOn()
            compose.onNodeWithText("Criar grupo fixo").assertIsEnabled().performScrollTo().performClick()
            try { compose.waitUntil(5000) { compose.onAllNodesWithText("Opções").fetchSemanticsNodes().isNotEmpty() } }
            catch (failure: Throwable) { throw AssertionError(compose.onRoot().printToString() + "\nVAULT=" + call(JSONObject().put("op", "view")), failure) }
            compose.onNodeWithText("Enviar").assertIsNotEnabled()
            compose.onNodeWithText("Opções").performClick()
            compose.onNodeWithText("Participantes e convite").performClick()
            compose.onNodeWithText("Grupo fixo: 3 participantes").assertExists()
            compose.onNodeWithText("Editar tagname de @bia").performScrollTo().performClick()
            compose.onNode(hasSetTextAction() and hasAnyAncestor(isDialog())).performTextReplacement("@amiga")
            compose.onNodeWithText("Salvar tagname").performClick()
            compose.onNodeWithText("Opções").performClick()
            compose.onNodeWithText("Apagar minhas mensagens desta conversa").performClick()
            compose.onNodeWithText("Apagar nesta conversa").performClick()
            compose.waitUntil(5000) { compose.onAllNodesWithText("Exclusão confirmada por 0 de 2 destinatários. Detalhes em Participantes e convite.").fetchSemanticsNodes().isNotEmpty() }
            val all = call(JSONObject().put("op", "view")).getJSONObject("view").getJSONArray("conversations")
            val group = (0 until all.length()).map { all.getJSONObject(it) }.first { it.optString("kind") == "group" }
            val active = call(JSONObject().put("op", "view").put("conversation", group.getString("id"))).getJSONObject("view").getJSONObject("active")
            val invitation = active.getJSONObject("group").getString("invitation")
            compose.onNodeWithText("← Conversas").performClick()
            compose.onNodeWithText("@amiga").assertExists()
            compose.onNodeWithText("Entrar em grupo").performClick()
            compose.onNodeWithText("Convite do grupo").performTextInput(invitation)
            compose.onNodeWithText("Conferir participantes").performScrollTo().performClick()
            compose.waitUntil(5000) { compose.onAllNodesWithText("Conferir composição do grupo").fetchSemanticsNodes().isNotEmpty() }
            compose.onNodeWithText("Conferi e aceito entrar").performClick()
            compose.waitUntil(5000) { compose.onAllNodesWithText("Opções").fetchSemanticsNodes().isNotEmpty() }
            assertEquals(3, call(JSONObject().put("op", "view")).getJSONObject("view").getJSONArray("conversations").length())
        } finally { key.fill(0); Native.nativeRevoke(); Native.nativeClose(); e.wipe() }
    }
}
