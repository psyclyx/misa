package org.misa.app

import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.text.input.VisualTransformation
import androidx.compose.ui.unit.dp

/** Local navigation: publications never open a request dialog. */
@Composable
fun WorkspaceControls(state: UiState, model: MisaViewModel) {
    var workspace by remember { mutableStateOf(state.instance.isEmpty()) }
    var views by remember { mutableStateOf(false) }
    var requests by remember(state.instance) { mutableStateOf(false) }
    var requestId by remember(state.instance) { mutableStateOf<String?>(null) }
    var panel by remember(state.instance) { mutableStateOf<String?>(null) }
    LaunchedEffect(state.instance, state.requestFocus) {
        state.requestFocus?.let {
            requestId = it
            model.consumeRequestFocus()
        }
    }
    Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceEvenly) {
        TextButton({ workspace = true }) { Text("Daemons") }
        TextButton({ views = true }) { Text("Views") }
        TextButton({ requests = true }, enabled = state.requests.isNotEmpty()) {
            Text("Requests (${state.requests.size})")
        }
    }
    state.renderProblem?.let { problem ->
        Text(
            problem,
            color = MaterialTheme.colorScheme.error,
            modifier = Modifier.padding(horizontal = 12.dp),
        )
        TextButton(model::retryRendering) { Text("Refresh presentation") }
    }
    state.panels["status"]?.let { status ->
        val fields = statusFields(status).filter { it.id !in state.hiddenStatus }
        CompositionLocalProvider(
            LocalStreams provides
                StreamDisplay(model.panelStreams("status"), { model.panelContains("status", it) })
        ) {
            key(state.instance, "status") {
                androidx.compose.foundation.lazy.LazyRow(
                    Modifier.fillMaxWidth().heightIn(max = 104.dp).padding(horizontal = 12.dp),
                    horizontalArrangement = Arrangement.spacedBy(12.dp),
                ) {
                    items(fields.size, key = { fields[it].id }) { index ->
                        val child = fields[index]
                        Box(Modifier.width(144.dp)) {
                            Transcript(
                                Node(
                                    "status.slot",
                                    "status",
                                    "",
                                    "",
                                    Shape.Section,
                                    emptyList(),
                                    listOf(child),
                                ),
                                emptySet(),
                                {},
                                model::action,
                            )
                        }
                    }
                }
            }
        }
    }
    if (workspace) {
        var ticket by remember { mutableStateOf("") }
        var scan by remember { mutableStateOf(false) }
        LocalSheet("Daemons and sessions", { workspace = false }) {
            OutlinedTextField(
                ticket,
                { ticket = it },
                label = { Text("Ticket or pairing code") },
                modifier = Modifier.fillMaxWidth(),
            )
            Row {
                TextButton(
                    {
                        model.connect(ticket)
                        ticket = ""
                    },
                    enabled = ticket.isNotBlank(),
                ) {
                    Text("Connect")
                }
                TextButton({ scan = !scan }) { Text("Scan") }
            }
            if (scan)
                Box(Modifier.height(240.dp)) {
                    QrScanner(
                        onCode = {
                            model.connect(it)
                            scan = false
                        },
                        onProblem = { scan = false },
                    )
                }
            Text(state.message)
            state.notices.takeLast(3).forEach { notice ->
                Text(
                    notice.text,
                    color =
                        if (notice.level == "error") MaterialTheme.colorScheme.error
                        else MaterialTheme.colorScheme.onSurface,
                )
            }
            state.daemons
                .flatMap { it.sessions }
                .flatMap { it.requests }
                .sortedBy { !it.available }
                .distinctBy { "${it.daemon}:${it.scope}:${it.id}" }
                .forEach { target ->
                    TextButton(
                        {
                            model.navigateRequest(target)
                            workspace = false
                        },
                        enabled = target.available,
                    ) {
                        Text(
                            "Open ${target.title} · ${target.daemon.take(8)} · ${target.scope.getJSONObject("id").getString("id")}${if (target.available) "" else " · stale"}"
                        )
                    }
                }
            state.daemons.forEach { daemon ->
                Text("${daemon.id.take(12)} · ${daemon.freshness}")
                TextButton(
                    { model.daemonCommands(daemon.id) },
                    enabled = daemon.freshness == "current",
                ) {
                    Text("Daemon commands")
                }
                state.daemonCommands[daemon.id].orEmpty().forEach { command ->
                    TextButton(
                        { model.daemonForm(daemon.id, command) },
                        enabled = daemon.freshness == "current",
                    ) {
                        Text(command)
                    }
                }
                daemon.work.forEach { work ->
                    Text(
                        "${work.id} · ${work.state}${if(work.blocking) " · blocking" else ""}${if(work.available) "" else " · stale"}"
                    )
                    Row {
                        TextButton(
                            { model.workForm(daemon.id, work, "operation.cancel") },
                            enabled = work.available,
                        ) {
                            Text("Cancel work…")
                        }
                        TextButton(
                            { model.workForm(daemon.id, work, "daemon.work.forget") },
                            enabled = work.available,
                        ) {
                            Text("Forget work…")
                        }
                    }
                }
                daemon.sessions.forEach { session ->
                    Row {
                        TextButton({
                            model.select(daemon.id, session.scope)
                            workspace = false
                        }) {
                            Text(session.title.ifEmpty { "Session" } + "\n" + session.summary)
                        }
                        TextButton({ model.closeSession(daemon.id, session.scope) }) {
                            Text("Close on daemon")
                        }
                    }
                }
                var name by remember(daemon.id) { mutableStateOf("") }
                var conversation by remember(daemon.id) { mutableStateOf("") }
                OutlinedTextField(name, { name = it }, label = { Text("New session ID") })
                OutlinedTextField(
                    conversation,
                    { conversation = it },
                    label = { Text("Saved conversation ID (optional)") },
                )
                TextButton({ model.conversations(daemon.id, conversation) }) {
                    Text("Browse saved conversations")
                }
                state.archives[daemon.id].orEmpty().forEach { (id, label) ->
                    TextButton({ conversation = id }) { Text(label) }
                }
                TextButton(
                    { model.createSession(daemon.id, name, conversation) },
                    enabled = name.isNotBlank(),
                ) {
                    Text(if (conversation.isBlank()) "Create session" else "Resume conversation")
                }
                TextButton({ model.disconnectDaemon(daemon.id) }) { Text("Disconnect daemon") }
            }
            state.instances.forEach { instance ->
                Row {
                    TextButton(
                        {
                            model.selectInstance(instance.id)
                            workspace = false
                        },
                        modifier = Modifier.weight(1f),
                    ) {
                        Text(
                            "${instance.title} · ${instance.daemon.take(8)}${if(instance.available)"" else " · retained locally"}"
                        )
                    }
                    TextButton({ model.closeInstance(instance.id) }) {
                        Text("Close · discard draft")
                    }
                }
            }
        }
    }
    if (views)
        LocalSheet("Presentations", { views = false }) {
            Text("Theme")
            Row {
                listOf("system", "dark", "light").forEach { theme ->
                    TextButton({ model.theme(theme) }, enabled = state.theme != theme) {
                        Text(theme)
                    }
                }
            }
            state.panels["status"]?.let { status ->
                Text("Status fields")
                statusFields(status).forEach { field ->
                    Row {
                        Checkbox(
                            field.id !in state.hiddenStatus,
                            { model.statusField(field.id, it) },
                        )
                        Text(field.label?.takeIf { it.isNotEmpty() } ?: field.id)
                    }
                }
            }
            state.presentations.forEach { choice ->
                Text("${choice.title} · ${choice.selected}")
                Row {
                    if (choice.id != "conversation")
                        TextButton({ model.presentation(choice.id, "hidden") }) { Text("Hide") }
                    TextButton({ model.presentation(choice.id, "auto") }) { Text("Auto") }
                    if (state.panels.containsKey(choice.id))
                        TextButton({
                            panel = choice.id
                            views = false
                        }) {
                            Text("Open")
                        }
                }
                choice.variants.forEach { variant ->
                    TextButton({ model.presentation(choice.id, "variant", variant) }) {
                        Text(variant)
                    }
                }
            }
        }
    if (requests)
        LocalSheet("Pending requests", { requests = false }) {
            state.requests.forEach { request ->
                TextButton({
                    requestId = request.id
                    requests = false
                }) {
                    Text(request.title)
                }
            }
        }
    state.requests
        .find { it.id == requestId }
        ?.let { request ->
            val draftKey = "${request.id}:${request.generation}"
            var secret by remember(state.instance, draftKey) { mutableStateOf("") }
            var draft by
                remember(state.instance, draftKey) { mutableStateOf(model.requestDraft(draftKey)) }
            val value = if (request.secret) secret else draft
            val values =
                remember(state.instance, draftKey) {
                    mutableStateMapOf<String, String>().apply {
                        request.fields.forEach {
                            put(it.id, model.requestDraft("$draftKey:${it.id}"))
                        }
                    }
                }
            LocalSheet(
                request.title,
                {
                    secret = ""
                    requestId = null
                },
            ) {
                key(state.instance, draftKey) {
                    Transcript(request.body, emptySet(), {}, { _, _, _ -> })
                }
                if (request.input != null)
                    OutlinedTextField(
                        value,
                        {
                            if (request.secret) secret = it
                            else {
                                draft = it
                                model.requestDraft(draftKey, it)
                            }
                        },
                        label = { Text(request.label) },
                        visualTransformation =
                            if (request.secret) PasswordVisualTransformation()
                            else VisualTransformation.None,
                        modifier = Modifier.fillMaxWidth(),
                    )
                request.fields.forEach { field ->
                    OutlinedTextField(
                        values[field.id].orEmpty(),
                        {
                            values[field.id] = it
                            model.requestDraft("$draftKey:${field.id}", it)
                        },
                        label = { Text(field.label + if (field.optional) " (optional)" else "") },
                        supportingText = { Text(if (field.text) "Text" else "JSON value") },
                        modifier = Modifier.fillMaxWidth(),
                    )
                }
                state.notices
                    .lastOrNull()
                    ?.takeIf { it.level == "error" }
                    ?.let { Text(it.text, color = MaterialTheme.colorScheme.error) }
                if (!state.connected) Text("Request owner unavailable; retained details are stale")
                request.actions.forEach { action ->
                    TextButton(
                        onClick = {
                            model.request(request, action.id, value, values.toMap())
                            secret = ""
                            if (action.id == "cancel") requestId = null
                        },
                        enabled = state.connected,
                    ) {
                        Text(action.label)
                    }
                }
            }
        }
    state.report?.let { report ->
        LocalSheet("Result", model::dismissReport) {
            Transcript(report, emptySet(), {}, { _, _, _ -> })
        }
    }
    state.form?.let { form ->
        val values =
            remember(state.instance, form.daemon, form.scope?.toString(), form.action, form.drafts) {
                mutableStateMapOf<String, String>().apply {
                    form.fields.forEach {
                        put(
                            it.id,
                            form.drafts[it.id]
                                ?: model.requestDraft(
                                    "form:${form.daemon.orEmpty()}:${form.action}:${it.id}"
                                ),
                        )
                    }
                }
            }
        LocalSheet(form.action, model::dismissForm) {
            form.fields.forEach { field ->
                OutlinedTextField(
                    values[field.id].orEmpty(),
                    {
                        values[field.id] = it
                        model.requestDraft(
                            "form:${form.daemon.orEmpty()}:${form.action}:${field.id}",
                            it,
                        )
                    },
                    label = { Text("${field.id}${if(field.optional)" (optional)" else ""}") },
                    supportingText = { Text(if (field.text) "Text" else "JSON value") },
                    modifier = Modifier.fillMaxWidth(),
                )
            }
            state.notices.lastOrNull()?.let { Text(it.text) }
            TextButton({ model.submitForm(form, values.toMap()) }) { Text("Submit") }
        }
    }
    panel?.let { id ->
        state.panels[id]?.let { document ->
            LocalSheet(id, { panel = null }) {
                CompositionLocalProvider(
                    LocalStreams provides
                        StreamDisplay(model.panelStreams(id), { model.panelContains(id, it) })
                ) {
                    key(state.instance, id) {
                        Transcript(document, emptySet(), {}, model::action)
                        UnplacedStreams()
                    }
                }
            }
        }
    }
}

private fun statusFields(node: Node): List<Node> =
    if (node.shape is Shape.Section) node.children else listOf(node)

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun LocalSheet(
    title: String,
    dismiss: () -> Unit,
    content: @Composable ColumnScope.() -> Unit,
) {
    ModalBottomSheet(onDismissRequest = dismiss) {
        Column(
            Modifier.fillMaxWidth().padding(16.dp).verticalScroll(rememberScrollState()),
            verticalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            Text(title, style = MaterialTheme.typography.titleLarge)
            content()
            TextButton(dismiss) { Text("Close") }
            Spacer(Modifier.height(24.dp))
        }
    }
}
