package org.misa.app

import android.content.Intent
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.SystemBarStyle
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.activity.result.contract.ActivityResultContracts
import androidx.activity.viewModels
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import kotlinx.coroutines.flow.MutableStateFlow

class MainActivity : ComponentActivity() {
    private val model: MisaViewModel by viewModels()

    override fun onStart() {
        super.onStart()
        model.resume()
    }

    override fun onStop() {
        model.pause()
        super.onStop()
    }

    /**
     * A ticket or pairing string the app was opened with.
     *
     * A `misa-pair:` URI is what the system camera produces from the daemon's QR, so a phone can
     * pair from the camera without this app ever being opened first. An extra carries the same
     * string, which is what makes the whole flow scriptable.
     */
    private val link = MutableStateFlow<String?>(null)

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge(statusBarStyle = SystemBarStyle.dark(android.graphics.Color.TRANSPARENT))
        link.value = fromIntent(intent)
        setContent {
            MaterialTheme(colorScheme = darkColorScheme()) {
                val state by model.state.collectAsStateWithLifecycle()
                val expanded by model.expanded.collectAsStateWithLifecycle()
                val opened by link.collectAsStateWithLifecycle()
                val upload =
                    rememberLauncherForActivityResult(ActivityResultContracts.OpenDocument()) { uri
                        ->
                        uri?.let(model::attach)
                    }
                val download =
                    rememberLauncherForActivityResult(
                        ActivityResultContracts.CreateDocument("application/octet-stream"),
                        model::saveDownload,
                    )
                LaunchedEffect(state.download) {
                    model.beginDownload()?.let { download.launch(it) }
                }
                CompositionLocalProvider(
                    androidx.compose.material3.LocalContentColor provides
                        MaterialTheme.colorScheme.onSurface,
                    LocalImages provides
                        ImageFiles(state.images, state.imageErrors, model::image, state.connected),
                    LocalStreams provides StreamDisplay(model.streams, model::containsNode),
                ) {
                    Column(
                        Modifier.fillMaxSize()
                            .background(MaterialTheme.colorScheme.background)
                            .statusBarsPadding()
                    ) {
                        WorkspaceControls(state, model)
                        androidx.compose.runtime.key(state.instance) {
                            Misa(
                                state = state,
                                expanded = expanded,
                                initial = opened,
                                onConnect = model::connect,
                                onDisconnect = model::disconnect,
                                onPrompt = model::prompt,
                                onCommand = model::command,
                                onComplete = model::complete,
                                onAction = model::action,
                                onCancel = model::cancel,
                                onToggle = model::toggle,
                                onDraft = model::draft,
                                onAttach = { upload.launch(arrayOf("*/*")) },
                                onRemoveAttachment = model::removeAttachment,
                            )
                        }
                    }
                }
            }
        }
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        fromIntent(intent)?.let { link.value = it }
    }

    private fun fromIntent(intent: Intent?): String? {
        val fromLink = intent?.data?.toString()?.takeIf { it.isNotEmpty() }
        return fromLink ?: intent?.getStringExtra("ticket")?.takeIf { it.isNotEmpty() }
    }
}

@Composable
private fun Misa(
    state: UiState,
    expanded: Set<String>,
    initial: String?,
    onConnect: (String) -> Unit,
    onDisconnect: () -> Unit,
    onPrompt: (String) -> Unit,
    onCommand: (String, List<Pair<String, String>>) -> Unit,
    onComplete: (String, String) -> Unit,
    onAction: (String, Action, List<FieldValue>) -> Unit,
    onCancel: () -> Unit,
    onToggle: (String) -> Unit,
    onDraft: (String) -> Unit,
    onAttach: () -> Unit,
    onRemoveAttachment: (Int) -> Unit,
) {
    // A link that opened the app is a ticket somebody already chose, so it connects
    // without a second tap. `initial` is stable, so this fires once.
    LaunchedEffect(initial) {
        if (!initial.isNullOrEmpty()) onConnect(initial)
    }
    if (state.instance.isEmpty()) return
    Session(
        state,
        expanded,
        onDisconnect,
        onPrompt,
        onCommand,
        onComplete,
        onAction,
        onCancel,
        onToggle,
        onDraft,
        onAttach,
        onRemoveAttachment,
        { onConnect(state.ticket) },
    )
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun Session(
    state: UiState,
    expanded: Set<String>,
    onDisconnect: () -> Unit,
    onPrompt: (String) -> Unit,
    onCommand: (String, List<Pair<String, String>>) -> Unit,
    onComplete: (String, String) -> Unit,
    onAction: (String, Action, List<FieldValue>) -> Unit,
    onCancel: () -> Unit,
    onToggle: (String) -> Unit,
    onDraft: (String) -> Unit,
    onAttach: () -> Unit,
    onRemoveAttachment: (Int) -> Unit,
    onReconnect: () -> Unit,
) {
    val list = rememberLazyListState()
    val draft = state.draft
    var palette by remember { mutableStateOf(false) }
    var arguments by remember { mutableStateOf<Command?>(null) }
    val commands = state.session?.commands ?: emptyList()
    var rows by remember { mutableStateOf(listOf<Node>()) }
    // The transcript is one flat list of top-level nodes, so the list can be lazy
    // while a single node may still be tall; a key is its id, so a streamed answer
    // re-renders in place rather than being replaced under the reader.
    LaunchedEffect(state.view) {
        // The application places the composer and owns its draft.
        rows =
            state.view?.children?.flatMap {
                when (it.role) {
                    "composer" -> emptyList()
                    "transcript" -> it.children
                    else -> listOf(it)
                }
            } ?: emptyList()
    }
    val streamedBytes = LocalStreams.current.streams.values.values.sumOf { it.bytes }
    var previousRows by remember { mutableStateOf(0) }
    LaunchedEffect(rows.size, streamedBytes) {
        val following =
            previousRows == 0 ||
                (list.layoutInfo.visibleItemsInfo.lastOrNull()?.index ?: 0) >= previousRows - 1
        previousRows = rows.size
        if (following) list.scrollToItem(rows.size)
    }
    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text(state.session?.title?.ifEmpty { state.session.id } ?: "session") },
                actions = {
                    if (!state.connected) TextButton(onClick = onReconnect) { Text("Reconnect") }
                    TextButton(onClick = { onCancel() }, enabled = state.connected) { Text("Stop") }
                    TextButton(onClick = { palette = true }) { Text("Commands") }
                    TextButton(onClick = { onDisconnect() }) { Text("Detach") }
                },
            )
        }
    ) { padding ->
        Column(Modifier.fillMaxSize().padding(padding).imePadding()) {
            if (!state.connected) Text(state.message, modifier = Modifier.padding(12.dp))

            if (state.notices.isNotEmpty()) {
                Notices(state.notices)
            }
            if (state.status.isNotEmpty()) {
                Text(
                    state.status,
                    style = MaterialTheme.typography.labelSmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    modifier = Modifier.fillMaxWidth().padding(horizontal = 12.dp),
                )
            }
            LazyColumn(
                state = list,
                modifier = Modifier.weight(1f).fillMaxWidth().padding(horizontal = 12.dp),
                verticalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                items(rows.size, key = { index -> rows[index].id.ifEmpty { "row.$index" } }) { index
                    ->
                    NodeRow(rows[index], expanded, onToggle, onAction)
                }
                item(key = "local.live") { UnplacedStreams() }
            }
            Row(Modifier.padding(horizontal = 12.dp)) {
                TextButton(onClick = onAttach, enabled = state.connected && !state.uploading) {
                    Text(if (state.uploading) "Uploading…" else "Attach file")
                }
                state.attachments.forEachIndexed { index, attachment ->
                    TextButton(onClick = { onRemoveAttachment(index) }) {
                        Text("${attachment.name} ×")
                    }
                }
            }
            Composer(
                draft = draft,
                onDraft = onDraft,
                canSend =
                    state.connected &&
                        !state.uploading &&
                        (draft.isNotBlank() || state.attachments.isNotEmpty()),
                onSend = {
                    val text = draft.trim()
                    val parsed = parseCommand(text, commands)
                    if (parsed != null) onCommand(parsed.first, parsed.second) else onPrompt(text)
                },
            )
        }
    }
    if (palette) {
        val sheet = rememberModalBottomSheetState()
        ModalBottomSheet(onDismissRequest = { palette = false }, sheetState = sheet) {
            Column(
                Modifier.fillMaxWidth()
                    .verticalScroll(rememberScrollState())
                    .padding(bottom = 24.dp)
            ) {
                commands.forEach { command ->
                    ListItem(
                        headlineContent = { Text("/${command.id}  ${command.label}") },
                        supportingContent = {
                            val where =
                                if (command.args.isEmpty()) command.description
                                else command.args.joinToString(" ") { it.label }
                            Text(where)
                        },
                        modifier =
                            Modifier.clickable {
                                palette = false
                                if (command.args.isEmpty()) onCommand(command.id, emptyList())
                                else arguments = command
                            },
                    )
                }
                if (commands.isEmpty()) {
                    ListItem(headlineContent = { Text("this session declares no commands") })
                }
            }
        }
    }
    arguments?.let { command ->
        ArgumentDialog(
            command = command,
            completions = state.completions,
            onComplete = onComplete,
            onDismiss = { arguments = null },
            onSubmit = { values ->
                arguments = null
                onCommand(command.id, values)
            },
        )
    }
}

@Composable
private fun Notices(notices: List<Notice>) {
    val recent = notices.takeLast(3)
    Column(Modifier.fillMaxWidth().padding(horizontal = 12.dp, vertical = 4.dp)) {
        recent.forEach { notice ->
            val colour =
                when (notice.level) {
                    "error" -> MaterialTheme.colorScheme.error
                    "warn" -> MaterialTheme.colorScheme.tertiary
                    else -> MaterialTheme.colorScheme.onSurfaceVariant
                }
            Text(notice.text, style = MaterialTheme.typography.labelSmall, color = colour)
        }
    }
}

@Composable
private fun NodeRow(
    node: Node,
    expanded: Set<String>,
    onToggle: (String) -> Unit,
    onAction: (String, Action, List<FieldValue>) -> Unit,
) {
    Box(Modifier.fillMaxWidth()) {
        Transcript(
            root = node,
            expanded = expanded,
            onToggle = onToggle,
            onAction = onAction,
        )
    }
}

@Composable
private fun Composer(
    draft: String,
    onDraft: (String) -> Unit,
    canSend: Boolean,
    onSend: () -> Unit,
) {
    Row(
        modifier = Modifier.fillMaxWidth().padding(10.dp),
        verticalAlignment = Alignment.Bottom,
        horizontalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        OutlinedTextField(
            value = draft,
            onValueChange = onDraft,
            label = { Text("message, or /command") },
            modifier = Modifier.weight(1f),
            minLines = 1,
            maxLines = 6,
        )
        Button(onClick = onSend, enabled = canSend) { Text("Send") }
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun ArgumentDialog(
    command: Command,
    completions: List<Pair<String, String>>,
    onComplete: (String, String) -> Unit,
    onDismiss: () -> Unit,
    onSubmit: (List<Pair<String, String>>) -> Unit,
) {
    var values by remember { mutableStateOf(command.args.associate { it.name to "" }) }
    var choosing by remember { mutableStateOf<String?>(null) }
    androidx.compose.material3.AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("/${command.id}") },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                Text(command.description, style = MaterialTheme.typography.bodySmall)
                command.args.forEach { arg ->
                    OutlinedTextField(
                        value = values[arg.name] ?: "",
                        onValueChange = {
                            values = values + (arg.name to it)
                            arg.source?.let { source ->
                                choosing = arg.name
                                onComplete(source, it)
                            }
                        },
                        label = { Text(arg.label + if (arg.required) " *" else "") },
                        modifier = Modifier.fillMaxWidth(),
                    )
                    if (arg.source != null)
                        TextButton({
                            choosing = arg.name
                            onComplete(arg.source, values[arg.name].orEmpty())
                        }) {
                            Text("Browse " + arg.label)
                        }
                    if (choosing == arg.name)
                        completions.take(8).forEach { (value, label) ->
                            TextButton({
                                values = values + (arg.name to value)
                                choosing = null
                            }) {
                                Text(label)
                            }
                        }
                }
            }
        },
        confirmButton = { TextButton(onClick = { onSubmit(values.toList()) }) { Text("Run") } },
        dismissButton = { TextButton(onClick = onDismiss) { Text("Cancel") } },
    )
}

/**
 * A typed line, as a command or a prompt.
 *
 * A leading slash names a command the session declared, which is the same rule every other frontend
 * uses; the rest of the line is the argument. A slash that names nothing is text, because somebody
 * who typed `/etc/hosts` was asking the model about a path, not invoking a command.
 */
private fun parseCommand(
    line: String,
    commands: List<Command>,
): Pair<String, List<Pair<String, String>>>? {
    if (!line.startsWith("/")) return null
    val body = line.drop(1)
    val name = body.takeWhile { !it.isWhitespace() }
    val command = commands.firstOrNull { it.id == name } ?: return null
    val rest = body.drop(name.length).trim()
    if (rest.isEmpty()) return command.id to emptyList()
    val values =
        if (command.args.size <= 1) {
            listOf((command.args.firstOrNull()?.name ?: "value") to rest)
        } else {
            val parts = rest.split(Regex("\\s+"))
            command.args.mapIndexed { index, arg -> arg.name to (parts.getOrNull(index) ?: "") }
        }
    return command.id to values
}
