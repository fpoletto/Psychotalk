package app.psychotalk

import android.content.ClipData
import android.content.ClipboardManager
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.view.WindowManager
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.compose.BackHandler
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.unit.dp
import org.json.JSONObject
import org.json.JSONArray
import java.io.File
import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit

enum class Stage { LOCKED, UNLOCKING, CONNECTING, ACTIVE, LOCKING, CLEANING }
data class Screen(val stage: Stage = Stage.LOCKED, val setup: Boolean = false, val ready: Boolean = false,
    val data: JSONObject? = null, val error: String = "", val level: String = "")

class MainActivity : ComponentActivity() {
    private val worker = Executors.newSingleThreadScheduledExecutor()
    private val main = Handler(Looper.getMainLooper())
    private val gate = Any()
    private var foreground = false
    private var generation = 0L
    @Volatile private var opened = false
    private lateinit var envelopes: Envelopes
    private var screen by mutableStateOf(Screen())
    private var groupPreview by mutableStateOf<JSONObject?>(null)
    private var selectedConversation by mutableStateOf<String?>(null)
    private val heartbeat = object : Runnable {
        override fun run() { synchronized(gate) { if (foreground) { Native.nativeHeartbeat(); main.postDelayed(this, 400) } } }
    }
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        window.addFlags(WindowManager.LayoutParams.FLAG_SECURE)
        envelopes = Envelopes(this)
        setContent { MaterialTheme { Surface(Modifier.fillMaxSize()) { App() } } }
        worker.execute {
            val ok = runCatching { envelopes.recover() }.isSuccess
            main.post { screen = Screen(setup = !envelopes.configured, ready = ok, error = if (ok) "" else "Limpeza ou armazenamento incompleto. Reinicie para tentar concluir; a rede permanece bloqueada.") }
        }
        worker.scheduleWithFixedDelay({ if (opened) refresh() }, 500, 500, TimeUnit.MILLISECONDS)
    }
    override fun onResume() {
        super.onResume()
        synchronized(gate) { foreground = true; generation++ }
        window.decorView.alpha = 1f
        main.post(heartbeat)
    }
    override fun onPause() {
        synchronized(gate) { foreground = false; generation++; opened = false; Native.nativeRevoke() }
        main.removeCallbacks(heartbeat)
        window.decorView.alpha = 0f
        groupPreview = null
        selectedConversation = null
        screen = screen.copy(stage = Stage.LOCKING, data = null, error = "")
        worker.execute { Native.nativeClose(); main.post { screen = screen.copy(stage = Stage.LOCKED, data = null) } }
        super.onPause()
    }
    override fun onDestroy() { worker.shutdown(); super.onDestroy() }
    private fun token() = synchronized(gate) { generation }
    private fun current(token: Long) = synchronized(gate) { foreground && generation == token }
    private fun refresh() {
        val token = token()
        val selected = selectedConversation
        val request = JSONObject().put("op", "view").also { if (selected != null) it.put("conversation", selected) }.toString()
        val response = runCatching { JSONObject(Native.nativeCommand(request)) }.getOrNull()
        main.post {
            if (current(token) && opened && selectedConversation == selected) {
                if (response == null || response.optBoolean("error")) {
                    opened = false; Native.nativeRevoke(); screen = screen.copy(stage = Stage.LOCKED, data = null, error = "Sessão encerrada. Desbloqueie novamente."); worker.execute { Native.nativeClose() }
                } else screen = screen.copy(stage = if (response.optString("network").startsWith("Tor pronto")) Stage.ACTIVE else Stage.CONNECTING, data = response)
            }
        }
    }
    private fun enter(normal: String, cleanup: String? = null) {
        val token = token(); screen = screen.copy(stage = Stage.UNLOCKING, error = "")
        val password = normal.toByteArray(Charsets.UTF_8)
        val other = cleanup?.toByteArray(Charsets.UTF_8)
        worker.execute {
            var key: ByteArray? = null
            try {
                envelopes.recover()
                if (other != null) envelopes.setup(password, other)
                val result = envelopes.unlock(password)
                if (result.cleanup) {
                    main.post { screen = Screen(stage = Stage.CLEANING, ready = false) }
                    envelopes.wipe()
                    main.post { screen = Screen(setup = true, ready = true) }
                    return@execute
                }
                key = result.key ?: error("invalid password")
                val binary = File(applicationInfo.nativeLibraryDir, "libtor.so").absolutePath
                val success = synchronized(gate) {
                    if (!foreground || generation != token) false
                    else Native.nativeOpen(envelopes.root.absolutePath, binary, key, false).also { opened = it }
                }
                check(success)
                val level = envelopes.securityLevel()
                main.post { if (current(token) && opened) screen = Screen(stage = Stage.CONNECTING, ready = true, level = level) }
                refresh()
            } catch (_: Exception) {
                synchronized(gate) { opened = false; Native.nativeRevoke() }; Native.nativeClose()
                main.post { if (current(token)) screen = Screen(setup = !envelopes.configured, ready = !File(envelopes.root, "wipe.intent").exists(), error = "Não foi possível abrir o cofre. Verifique a senha; se uma limpeza estiver pendente, reinicie para concluí-la.") }
            } finally { password.fill(0); other?.fill(0); key?.fill(0) }
        }
    }
    private fun lockSession() {
        synchronized(gate) { generation++; opened = false; Native.nativeRevoke() }
        groupPreview = null
        selectedConversation = null
        screen = screen.copy(stage = Stage.LOCKING, data = null)
        worker.execute { Native.nativeClose(); main.post { screen = screen.copy(stage = Stage.LOCKED) } }
    }
    private fun selectConversation(id: String?) {
        groupPreview = null
        selectedConversation = id
        screen = screen.copy(data = null, error = "")
        worker.execute { refresh() }
    }
    private fun command(op: String, field: String? = null, value: String? = null, tagname: String? = null, extra: JSONObject? = null) {
        val token = token(); val selected = selectedConversation
        val request = JSONObject().put("op", op).also {
            if (selected != null) it.put("conversation", selected)
            if (field != null) it.put(field, value)
            if (tagname != null) it.put("tagname", tagname)
            extra?.keys()?.forEach { key -> it.put(key, extra.get(key)) }
        }.toString()
        worker.execute {
            val result = runCatching { JSONObject(Native.nativeCommand(request)) }.getOrNull()
            val ok = result != null && !result.optBoolean("error")
            main.post {
                if (current(token)) {
                    if (!ok) screen = screen.copy(error = "Operação recusada. Confira os códigos, os participantes, o nome e a sessão.")
                    else if (op == "preview_group") groupPreview = result!!.getJSONObject("preview")
                    else if (selectedConversation == selected) {
                        if (op in listOf("pair", "create_group", "import_group")) selectedConversation = result!!.getJSONObject("view").getJSONObject("active").getString("id")
                        screen = screen.copy(data = result, error = "")
                    }
                }
            }
        }
    }
    @Composable private fun App() {
        val s = screen
        val unlocked = s.stage == Stage.ACTIVE || s.stage == Stage.CONNECTING
        var password by remember(s.stage) { mutableStateOf("") }
        var confirm by remember(s.stage) { mutableStateOf("") }
        var cleanup by remember(s.stage) { mutableStateOf("") }
        var cleanupConfirm by remember(s.stage) { mutableStateOf("") }
        var adding by remember(unlocked) { mutableStateOf(false) }
        var groupMode by remember(unlocked, selectedConversation) { mutableStateOf("") }
        var groupMembers by remember(unlocked, selectedConversation) { mutableStateOf(setOf<String>()) }
        var membersDialog by remember(unlocked, selectedConversation) { mutableStateOf(false) }
        var renamePin by remember(unlocked, selectedConversation) { mutableStateOf("") }
        var code by remember(unlocked, selectedConversation) { mutableStateOf("") }
        var tagname by remember(unlocked, selectedConversation) { mutableStateOf("") }
        var text by remember(unlocked, selectedConversation) { mutableStateOf("") }
        var pairDialog by remember(unlocked, selectedConversation) { mutableStateOf(false) }
        var deleteDialog by remember(unlocked, selectedConversation) { mutableStateOf(false) }
        var renameDialog by remember(unlocked, selectedConversation) { mutableStateOf(false) }
        var menu by remember(unlocked, selectedConversation) { mutableStateOf(false) }
        var publicCodeDialog by remember(unlocked) { mutableStateOf(false) }
        val view = s.data?.optJSONObject("view")
        val chat = view?.optJSONObject("active")?.takeIf { it.optString("id") == selectedConversation }
        val group = chat?.optJSONObject("group")
        val groupReady = group == null || group.optInt("ready") == group.optInt("total")
        val ownCode = view?.optString("own_code", "")?.takeUnless { it == "null" }.orEmpty()
        val epoch = s.data?.optLong("epoch") ?: 0
        SideEffect {
            if (unlocked && s.data != null) worker.execute { Native.nativeCommand(JSONObject().put("op", "displayed").put("epoch", epoch).toString()) }
        }
        BackHandler(enabled = unlocked && (adding || groupMode.isNotEmpty() || selectedConversation != null)) {
            if (groupMode.isNotEmpty()) { groupMode = ""; groupPreview = null } else if (adding) adding = false else selectConversation(null)
        }
        val base = Modifier.fillMaxSize().systemBarsPadding().imePadding().padding(20.dp)
        Column(if (unlocked) base else base.verticalScroll(rememberScrollState()), verticalArrangement = Arrangement.spacedBy(12.dp)) {
            if (!unlocked || selectedConversation == null) Text("Psychotalk", style = MaterialTheme.typography.headlineMedium)
            if (s.error.isNotEmpty()) Text(s.error, color = MaterialTheme.colorScheme.error)
            if (!unlocked) {
                Text(if (s.setup) "Configure as duas senhas" else "Desbloqueie seu cofre")
                Text("Experimental. Guarde as senhas: não há recuperação.", style = MaterialTheme.typography.bodySmall)
                if (s.stage == Stage.LOCKED && s.ready) {
                    PasswordField(if (s.setup) "Senha normal" else "Senha", password) { password = it }
                    if (s.setup) {
                        PasswordField("Confirme a senha normal", confirm) { confirm = it }
                        PasswordField("Senha de limpeza local", cleanup) { cleanup = it }
                        PasswordField("Confirme a senha de limpeza", cleanupConfirm) { cleanupConfirm = it }
                        Text("A senha de limpeza destrói todo este cofre, incluindo todas as conversas. Ela não apaga cópias nos contatos.", style = MaterialTheme.typography.bodySmall)
                    }
                    val valid = password.isNotEmpty() && password.toByteArray().size <= 1024 && (!s.setup || (password == confirm && cleanup == cleanupConfirm && cleanup.isNotEmpty() && cleanup != password && cleanup.toByteArray().size <= 1024))
                    Button(onClick = { val p = password; val c = cleanup; password = ""; confirm = ""; cleanup = ""; cleanupConfirm = ""; enter(p, if (s.setup) c else null) }, enabled = valid) { Text(if (s.setup) "Criar cofre" else "Desbloquear") }
                } else { CircularProgressIndicator(); Text(when (s.stage) { Stage.CLEANING -> "Concluindo limpeza local…"; Stage.LOCKING -> "Encerrando sessão…"; else -> "Validando cofre…" }) }
            } else {
                if (selectedConversation == null) Text(s.data?.optString("network") ?: "Conectando ao Tor", style = MaterialTheme.typography.bodySmall)
                if (groupMode.isNotEmpty()) {
                    TextButton(onClick = { groupMode = ""; groupPreview = null }) { Text("← Conversas") }
                    Column(Modifier.weight(1f).verticalScroll(rememberScrollState()), verticalArrangement = Arrangement.spacedBy(12.dp)) {
                        if (groupMode == "create") {
                            Text("Novo grupo", style = MaterialTheme.typography.titleLarge)
                            Text("Escolha de 2 a 4 contatos. Com você, o grupo terá de 3 a 5 participantes fixos. Para mudar o nome ou a composição, será necessário criar outro grupo.")
                            OutlinedTextField(tagname, { if (it.codePointCount(0,it.length) <= 64) tagname = it }, label = { Text("Nome fixo do grupo") }, modifier = Modifier.fillMaxWidth(), singleLine = true)
                            val contacts = view?.optJSONArray("conversations")
                            for (i in 0 until (contacts?.length() ?: 0)) {
                                val c = contacts!!.getJSONObject(i)
                                if (c.optString("kind") != "group") {
                                    val id = c.getString("id")
                                    Row(verticalAlignment = Alignment.CenterVertically) {
                                        Checkbox(modifier = Modifier.semantics { contentDescription = "Selecionar ${c.getString("tagname")}" }, checked = id in groupMembers, enabled = id in groupMembers || groupMembers.size < 4, onCheckedChange = { checked -> groupMembers = if (checked) groupMembers + id else groupMembers - id })
                                        Column { Text(c.getString("tagname")); Text(c.getString("fingerprint").take(16) + "…", style = MaterialTheme.typography.labelSmall) }
                                    }
                                }
                            }
                            Text("O convite revela os endereços onion e as impressões digitais de todos aos participantes. Cada um deverá conferir e aceitar a mesma composição no próprio aparelho.")
                            Button(enabled = groupMembers.size in 2..4 && tagname.isNotBlank() && ownCode.isNotEmpty(), onClick = {
                                command("create_group", tagname = tagname, extra = JSONObject().put("members", JSONArray(groupMembers.toList())))
                                groupMode = ""
                            }) { Text("Criar grupo fixo") }
                        } else {
                            Text("Entrar em grupo", style = MaterialTheme.typography.titleLarge)
                            Text("Cole o convite recebido por um canal confiável. Seu código público precisa estar na composição. A entrada só ocorre após sua confirmação.")
                            OutlinedTextField(code, { code = it.take(8192) }, label = { Text("Convite do grupo") }, modifier = Modifier.fillMaxWidth(), maxLines = 6)
                            Button(enabled = code.isNotBlank(), onClick = { command("preview_group", "code", code) }) { Text("Conferir participantes") }
                        }
                    }
                } else if (adding) {
                    TextButton(onClick = { adding = false }) { Text("← Conversas") }
                    Column(Modifier.weight(1f).verticalScroll(rememberScrollState()), verticalArrangement = Arrangement.spacedBy(12.dp)) {
                        Text("Adicionar contato", style = MaterialTheme.typography.titleLarge)
                        Text("Troquem os códigos por um canal confiável e confirmem as impressões digitais nos dois aparelhos.")
                        OutlinedTextField(tagname, { if (it.codePointCount(0, it.length) <= 64) tagname = it }, label = { Text("Tagname local (opcional)") }, singleLine = true, modifier = Modifier.fillMaxWidth())
                        Text("Este nome fica somente no seu cofre.", style = MaterialTheme.typography.bodySmall)
                        OutlinedTextField(code, { code = it.take(180) }, label = { Text("Código público do contato") }, modifier = Modifier.fillMaxWidth())
                        Button(onClick = { pairDialog = true }, enabled = code.isNotBlank()) { Text("Verificar impressão digital") }
                        TextButton(onClick = { publicCodeDialog = true }) { Text("Meu código público") }
                    }
                } else if (selectedConversation == null) {
                    Text("Conversas", style = MaterialTheme.typography.titleLarge)
                    FlowRow(horizontalArrangement = Arrangement.spacedBy(12.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                        Button(onClick = { code = ""; tagname = ""; adding = true }) { Text("Adicionar contato") }
                        TextButton(onClick = { code = ""; tagname = ""; groupMembers = emptySet(); groupMode = "create" }) { Text("Novo grupo") }
                        TextButton(onClick = { code = ""; groupMode = "import" }) { Text("Entrar em grupo") }
                        TextButton(onClick = { publicCodeDialog = true }) { Text("Meu código") }
                    }
                    val conversations = view?.optJSONArray("conversations")
                    LazyColumn(Modifier.weight(1f).fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(12.dp)) {
                        if (conversations?.length() == 0) item { Text("Nenhuma conversa ainda. Adicione um contato para começar.") }
                        items((0 until (conversations?.length() ?: 0)).toList(), key = { conversations!!.getJSONObject(it).getString("id") }) { i ->
                            val conversation = conversations!!.getJSONObject(i)
                            val id = conversation.getString("id")
                            Card(onClick = { selectConversation(id) }, modifier = Modifier.fillMaxWidth()) {
                                Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
                                    Text(conversation.getString("tagname"), style = MaterialTheme.typography.titleMedium)
                                    Text(if (conversation.optString("kind") == "group") "Grupo fixo • via Tor" else s.data?.optJSONObject("connections")?.optString(id, "Offline") ?: "Offline", style = MaterialTheme.typography.bodySmall)
                                    Text("Impressão: ${conversation.getString("fingerprint").take(12)}…", style = MaterialTheme.typography.labelSmall)
                                    if (conversation.optInt("pending") > 0) Text("Entregas pendentes: ${conversation.getInt("pending")}", style = MaterialTheme.typography.bodySmall)
                                    if (conversation.optBoolean("deletion_pending")) Text("Exclusão aguardando confirmação", style = MaterialTheme.typography.bodySmall)
                                }
                            }
                        }
                    }
                } else {
                    FlowRow(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceBetween, verticalArrangement = Arrangement.spacedBy(8.dp)) {
                        TextButton(onClick = { selectConversation(null) }) { Text("← Conversas") }
                        Box {
                            TextButton(onClick = { menu = true }, enabled = chat != null) { Text("Opções") }
                            DropdownMenu(expanded = menu, onDismissRequest = { menu = false }) {
                                DropdownMenuItem(text = { Text("Bloquear") }, onClick = { menu = false; lockSession() })
                                if (group == null) DropdownMenuItem(text = { Text("Editar tagname") }, onClick = { menu = false; renamePin = ""; tagname = chat!!.getString("tagname"); renameDialog = true })
                                else DropdownMenuItem(text = { Text("Participantes e convite") }, onClick = { menu = false; membersDialog = true })
                                DropdownMenuItem(text = { Text("Apagar minhas mensagens desta conversa") }, onClick = { menu = false; deleteDialog = true })
                                DropdownMenuItem(text = { Text("Meu código público") }, onClick = { menu = false; publicCodeDialog = true })
                            }
                        }
                    }
                    if (chat == null) {
                        Box(Modifier.weight(1f).fillMaxWidth(), contentAlignment = Alignment.Center) { CircularProgressIndicator() }
                    } else {
                        Text(chat.getString("tagname"), style = MaterialTheme.typography.titleLarge, maxLines = 2, overflow = androidx.compose.ui.text.style.TextOverflow.Ellipsis)
                        Text("${s.data?.optString("network") ?: "Conectando ao Tor"} • ${if (group != null) "${group.optInt("ready")} de ${group.optInt("total")} participantes prontos" else s.data?.optJSONObject("connections")?.optString(selectedConversation!!, "Offline") ?: "Offline"}", style = MaterialTheme.typography.bodySmall)
                        if (group != null && !groupReady) Text("Aguardando aceitação e sincronização de todos. Mantenham os aplicativos abertos e desbloqueados.", style = MaterialTheme.typography.bodySmall)
                        val deletion = group?.optJSONObject("deletion")
                        if (deletion != null) Text("Exclusão confirmada por ${deletion.optInt("confirmed")} de ${deletion.optInt("total")} destinatários. Detalhes em Participantes e convite.", style = MaterialTheme.typography.bodySmall)
                        else if (group == null && !chat.isNull("deletion")) Text(if (chat.optBoolean("deletion")) "Exclusão confirmada pelo aplicativo do contato" else "Exclusão pendente no outro aparelho", style = MaterialTheme.typography.bodySmall)
                        val messages = chat.getJSONArray("messages")
                        LazyColumn(Modifier.weight(1f).fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(10.dp)) {
                            items((0 until messages.length()).toList()) { i -> MessageBubble(messages.getJSONObject(i), chat.getString("tagname")) }
                        }
                        Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.Bottom) {
                            OutlinedTextField(text, { if (it.toByteArray(Charsets.UTF_8).size <= 4096) text = it }, label = { Text("Mensagem de texto") }, modifier = Modifier.weight(1f), maxLines = 4)
                            Button(onClick = { command("send", "text", text); text = "" }, enabled = text.isNotEmpty() && groupReady) { Text("Enviar") }
                        }
                    }
                }
                if (selectedConversation == null) {
                    Text(s.level, style = MaterialTheme.typography.labelSmall)
                    TextButton(onClick = { lockSession() }) { Text("Bloquear") }
                }
            }
        }
        if (unlocked && groupPreview != null) AlertDialog(onDismissRequest = { groupPreview = null }, title = { Text("Conferir composição do grupo") },
            text = { Column(Modifier.verticalScroll(rememberScrollState()), verticalArrangement = Arrangement.spacedBy(12.dp)) {
                Text(groupPreview!!.getString("name"))
                Text("Impressão do grupo: ${groupPreview!!.getString("id")}")
                Text("Confirme esta impressão e cada participante por um canal confiável. Todos terão acesso aos códigos públicos abaixo.")
                val members = groupPreview!!.getJSONArray("members")
                for (i in 0 until members.length()) Text(members.getJSONObject(i).getString("code"), style = MaterialTheme.typography.bodySmall)
            } },
            confirmButton = { TextButton(onClick = { command("import_group", "code", code); groupPreview = null; code = ""; groupMode = "" }) { Text("Conferi e aceito entrar") } },
            dismissButton = { TextButton(onClick = { groupPreview = null }) { Text("Cancelar") } })
        if (membersDialog && group != null) AlertDialog(onDismissRequest = { membersDialog = false }, title = { Text("Participantes e convite") },
            text = { Column(Modifier.verticalScroll(rememberScrollState()), verticalArrangement = Arrangement.spacedBy(12.dp)) {
                Text("Grupo fixo: ${group.getInt("total")} participantes")
                Text("Impressão do grupo: ${group.getString("fingerprint")}", style = MaterialTheme.typography.bodySmall)
                val members = group.getJSONArray("members")
                val receipts = group.optJSONObject("deletion")?.optJSONArray("recipients")
                for (i in 0 until members.length()) {
                    val member = members.getJSONObject(i)
                    val mine = member.getString("code") == ownCode
                    Text(member.getString("name"), style = MaterialTheme.typography.titleSmall)
                    Text(if (member.optBoolean("ready")) "Pronto" else if (member.optBoolean("joined")) "Aceitou • sincronizando" else "Aguardando aceitação", style = MaterialTheme.typography.bodySmall)
                    Text(member.getString("code"), style = MaterialTheme.typography.labelSmall)
                    for (r in 0 until (receipts?.length() ?: 0)) {
                        val receipt = receipts!!.getJSONObject(r)
                        if (receipt.getString("fingerprint") == member.getString("fingerprint")) Text(if (receipt.getBoolean("confirmed")) "Exclusão confirmada por este aplicativo" else "Exclusão pendente neste destinatário", style = MaterialTheme.typography.bodySmall)
                    }
                    if (!mine) TextButton(onClick = { renamePin = member.getString("fingerprint"); tagname = member.getString("name"); membersDialog = false; renameDialog = true }) { Text("Editar tagname de ${member.getString("name")}") }
                }
                Text("O convite só funciona para estas identidades. Compartilhe por um canal confiável; ele contém os códigos públicos de todos.", style = MaterialTheme.typography.bodySmall)
            } },
            confirmButton = { TextButton(onClick = { getSystemService(ClipboardManager::class.java).setPrimaryClip(ClipData.newPlainText("Convite público Psychotalk", group.getString("invitation"))); membersDialog = false }) { Text("Copiar convite") } },
            dismissButton = { TextButton(onClick = { membersDialog = false }) { Text("Fechar") } })
        if (pairDialog) AlertDialog(onDismissRequest = { pairDialog = false }, title = { Text("Confirmar impressão digital") },
            text = { Text("Confirme com o contato pelo canal confiável:\n\n${code.substringAfterLast(':')}\n\nAmbos devem confirmar no próprio aparelho.") },
            confirmButton = { TextButton(onClick = { command("pair", "code", code, tagname); code = ""; tagname = ""; adding = false; pairDialog = false }) { Text("Confirmei com o contato") } },
            dismissButton = { TextButton(onClick = { pairDialog = false }) { Text("Cancelar") } })
        if (renameDialog) AlertDialog(onDismissRequest = { renameDialog = false }, title = { Text("Editar tagname") },
            text = { Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
                OutlinedTextField(tagname, { if (it.codePointCount(0, it.length) <= 64) tagname = it }, label = { Text("Tagname local") }, singleLine = true)
                Text("Visível apenas neste aparelho. Deixe vazio para usar a identificação abreviada.")
            } },
            confirmButton = { TextButton(onClick = { if (renamePin.isEmpty()) command("rename", "tagname", tagname) else command("rename_member", "tagname", tagname, extra = JSONObject().put("pin", renamePin)); renameDialog = false }) { Text("Salvar tagname") } },
            dismissButton = { TextButton(onClick = { renameDialog = false }) { Text("Cancelar") } })
        if (deleteDialog) AlertDialog(onDismissRequest = { deleteDialog = false }, title = { Text("Apagar suas mensagens desta conversa?") },
            text = { Text("Remove tudo que você enviou até agora somente nesta conversa. ${if (group != null) "Cada destinatário confirmará separadamente após aplicar a exclusão. Quem estiver offline permanecerá pendente." else "O aplicativo deste contato aplicará o pedido quando ambos estiverem abertos e desbloqueados."} Capturas, cópias externas e clientes modificados ficam fora desse mecanismo.") },
            confirmButton = { TextButton(onClick = { command("delete"); deleteDialog = false }) { Text("Apagar nesta conversa") } },
            dismissButton = { TextButton(onClick = { deleteDialog = false }) { Text("Cancelar") } })
        if (publicCodeDialog) AlertDialog(onDismissRequest = { publicCodeDialog = false }, title = { Text("Meu código público") },
            text = { Text(if (ownCode.isEmpty()) "Preparando o endereço onion…" else "$ownCode\n\nEste código é o mesmo para todos os seus contatos.") },
            confirmButton = { TextButton(enabled = ownCode.isNotEmpty(), onClick = { getSystemService(ClipboardManager::class.java).setPrimaryClip(ClipData.newPlainText("Contato público Psychotalk", ownCode)); publicCodeDialog = false }) { Text("Copiar código") } },
            dismissButton = { TextButton(onClick = { publicCodeDialog = false }) { Text("Fechar") } })
    }
    @Composable private fun MessageBubble(message: JSONObject, contactName: String) {
        val mine = message.getBoolean("mine")
        Box(Modifier.fillMaxWidth().padding(start = if (mine) 48.dp else 0.dp, end = if (mine) 0.dp else 48.dp), contentAlignment = if (mine) Alignment.CenterEnd else Alignment.CenterStart) {
            Card(Modifier.widthIn(max = 320.dp), colors = CardDefaults.cardColors(containerColor = if (mine) MaterialTheme.colorScheme.primaryContainer else MaterialTheme.colorScheme.surfaceVariant, contentColor = if (mine) MaterialTheme.colorScheme.onPrimaryContainer else MaterialTheme.colorScheme.onSurfaceVariant)) {
                Column(Modifier.padding(12.dp)) {
                    Text(if (mine) "Você" else message.optString("author", contactName), style = MaterialTheme.typography.labelSmall)
                    Text(message.getString("text"))
                    if (mine) Text(if (message.optInt("recipient_count") > 1) "Entregue a ${message.optInt("delivered_count")} de ${message.optInt("recipient_count")}" else if (message.getBoolean("delivered")) "Entregue" else "Pendente", style = MaterialTheme.typography.labelSmall)
                }
            }
        }
    }
    @Composable private fun PasswordField(label: String, value: String, change: (String) -> Unit) {
        OutlinedTextField(value, change, label = { Text(label) }, visualTransformation = PasswordVisualTransformation(),
            keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Password, autoCorrectEnabled = false), singleLine = true, modifier = Modifier.fillMaxWidth())
    }
}
