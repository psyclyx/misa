package org.misa.app

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Button
import androidx.compose.material3.Checkbox
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.withStyle
import androidx.compose.ui.unit.dp
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.produceState
import androidx.compose.foundation.Image
import androidx.compose.ui.graphics.asImageBitmap
import android.graphics.BitmapFactory
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

internal data class ImageFiles(val paths: Map<String, String>, val errors: Map<String, String>, val fetch: (String) -> Unit, val connected: Boolean = false)
internal val LocalImages = staticCompositionLocalOf { ImageFiles(emptyMap(), emptyMap(), {}) }


/**
 * A view tree, drawn.
 *
 * The session decides structure and never appearance; this decides appearance
 * and nothing else, which is the same division the terminal's cells and the
 * browser's HTML are on the other side of. A role maps to a colour and a weight
 * here, exactly as a role maps to a class in the stylesheet — and an unfamiliar
 * role draws legibly rather than being dropped.
 */
@Composable
fun Transcript(root: Node, expanded: Set<String>, onToggle: (String) -> Unit, onAction: (String, Action, List<FieldValue>) -> Unit) {
    Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
        root.children.forEach { NodeView(it, expanded, onToggle, onAction) }
    }
}

@Composable
private fun NodeView(node: Node, expanded: Set<String>, onToggle: (String) -> Unit, onAction: (String, Action, List<FieldValue>) -> Unit) {
    val surface = roleSurface(node.role)
    Column(
        modifier =
            surface?.let {
                Modifier
                    .fillMaxWidth()
                    .background(it, RoundedCornerShape(10.dp))
                    .padding(horizontal = 10.dp, vertical = 8.dp)
            } ?: Modifier.fillMaxWidth(),
        verticalArrangement = Arrangement.spacedBy(4.dp),
    ) {
        node.label?.let {
            Text(it, style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
        }
        ShapeView(node, expanded, onToggle, onAction)
        // A node's own click actions are buttons under its content; a submit action
        // inside a form is drawn by the form itself, because it belongs to the form.
        if (node.shape !is Shape.Fields) {
            node.actions.filter { it.on == "click" }.forEach { action ->
                ActionButton(node.id, action, onAction)
            }
        }
    }
}

@Composable
private fun ShapeView(node: Node, expanded: Set<String>, onToggle: (String) -> Unit, onAction: (String, Action, List<FieldValue>) -> Unit) {
    when (val shape = node.shape) {
        is Shape.Section -> {
            node.children.forEach { NodeView(it, expanded, onToggle, onAction) }
            if (node.id == "transcript") StreamText()
        }
        is Shape.Text -> Text(inline(shape.spans), style = bodyFor(node.role))
        // A heading's level picks a type size; a quote gets a marker of the medium's own and
        // its blocks behind it; a rule is a row of the same character. The session said only
        // *what* each of those is.
        is Shape.Heading -> Text(inline(shape.spans), style = headingFor(shape.level), fontWeight = FontWeight.SemiBold)
        is Shape.Quote ->
            Row {
                Text("▏", color = MaterialTheme.colorScheme.onSurfaceVariant)
                Column(Modifier.fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(4.dp)) {
                    node.children.forEach { NodeView(it, expanded, onToggle, onAction) }
                }
            }
        is Shape.Rule -> Text("─".repeat(24), color = MaterialTheme.colorScheme.onSurfaceVariant)
        is Shape.Code -> CodeBlock(shape)
        is Shape.Bullets -> Bullets(shape, node, expanded, onToggle, onAction)
        is Shape.Table -> DataTable(shape)
        is Shape.Fields -> Form(node, shape, onAction)
        is Shape.Collapsible -> Collapsible(node, shape, expanded, onToggle, onAction)
        is Shape.Picture -> Picture(shape)
        is Shape.Status -> Text(shape.text, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
        is Shape.Meter ->
            Column {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    LinearProgressIndicator(
                        progress = { (shape.value / shape.max.coerceAtLeast(1.0)).toFloat().coerceIn(0f, 1f) },
                        modifier = Modifier.width(120.dp),
                    )
                    Text("  ${shape.value}/${shape.max}", style = MaterialTheme.typography.bodySmall)
                }
                if (shape.meterLabel.isNotEmpty()) {
                    Text(shape.meterLabel, style = MaterialTheme.typography.labelSmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                }
            }
        is Shape.Fact -> Text(Facts.format(node.role, shape.value), style = bodyFor(node.role), fontWeight = FontWeight.SemiBold)
        // A shape this build has never heard of, drawn as its children rather than
        // dropped: the vocabulary is open, and an addition must not be an outage.
        is Shape.Unknown -> node.children.forEach { NodeView(it, expanded, onToggle, onAction) }
    }
}

@Composable
private fun CodeBlock(shape: Shape.Code) {
    Column(
        Modifier
            .fillMaxWidth()
            .background(MaterialTheme.colorScheme.surfaceVariant, RoundedCornerShape(8.dp))
            .padding(8.dp),
    ) {
        shape.lang?.let {
            Text(it, style = MaterialTheme.typography.labelSmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
        }
        Text(highlight(shape), style = MaterialTheme.typography.bodySmall.copy(fontFamily = FontFamily.Monospace))
    }
}

@Composable
private fun Bullets(shape: Shape.Bullets, node: Node, expanded: Set<String>, onToggle: (String) -> Unit, onAction: (String, Action, List<FieldValue>) -> Unit) {
    Column(verticalArrangement = Arrangement.spacedBy(2.dp)) {
        shape.items.forEachIndexed { position, item ->
            Row {
                Text(
                    if (shape.ordered) "${position + 1}." else "•",
                    modifier = Modifier.width(22.dp),
                    style = MaterialTheme.typography.bodyMedium,
                )
                Column(Modifier.fillMaxWidth()) {
                    item.forEach { NodeView(it, expanded, onToggle, onAction) }
                }
            }
        }
    }
    if (node.children.isNotEmpty()) {
        node.children.forEach { NodeView(it, expanded, onToggle, onAction) }
    }
}

@Composable
private fun DataTable(shape: Shape.Table) {
    Column(Modifier.fillMaxWidth()) {
        if (shape.head.isNotEmpty()) {
            Row(Modifier.fillMaxWidth().background(MaterialTheme.colorScheme.surfaceVariant).padding(4.dp)) {
                shape.head.forEach { cell ->
                    Text(inline(cell), style = MaterialTheme.typography.labelMedium, modifier = Modifier.weight(1f))
                }
            }
        }
        shape.rows.forEach { row ->
            Row(Modifier.fillMaxWidth().padding(4.dp)) {
                row.forEach { cell ->
                    Text(inline(cell), style = MaterialTheme.typography.bodySmall, modifier = Modifier.weight(1f))
                }
            }
        }
    }
}

@Composable
private fun Collapsible(node: Node, shape: Shape.Collapsible, expanded: Set<String>, onToggle: (String) -> Unit, onAction: (String, Action, List<FieldValue>) -> Unit) {
    val open = node.id in expanded
    Column {
        TextButton(onClick = { if (node.id.isNotEmpty()) onToggle(node.id) }, enabled = node.id.isNotEmpty()) {
            Text((if (open) "▾ " else "▸ ") + inline(shape.summary).text, style = MaterialTheme.typography.bodyMedium)
        }
        if (open) {
            Column(Modifier.padding(start = 12.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
                node.children.forEach { NodeView(it, expanded, onToggle, onAction) }
            }
        }
    }
}

@Composable
private fun Form(node: Node, shape: Shape.Fields, onAction: (String, Action, List<FieldValue>) -> Unit) {
    val values = remember(node.id) { mutableStateMapOf<String, String>() }
    val submit = node.actionOn("submit")
    Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
        shape.fields.forEach { field ->
            val current = values[field.id] ?: field.value
            when {
                // A row rather than an input: a panel's facts are read, and a text field
                // anybody could type into would be an edit nothing can save.
                field.readOnly ->
                    Column {
                        Text(field.label, style = MaterialTheme.typography.labelMedium)
                        Text(if (field.secret) "••••" else field.value, style = MaterialTheme.typography.bodyMedium)
                    }

                field.secret -> OutlinedTextField(
                    value = current,
                    onValueChange = { values[field.id] = it },
                    label = { Text(field.label) },
                    visualTransformation = androidx.compose.ui.text.input.PasswordVisualTransformation(),
                    modifier = Modifier.fillMaxWidth(),
                )
                field.shape == "block" ->
                    OutlinedTextField(
                        value = current,
                        onValueChange = { values[field.id] = it },
                        label = { Text(field.label) },
                        modifier = Modifier.fillMaxWidth(),
                        minLines = 2,
                    )
                field.shape == "bool" ->
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        Checkbox(
                            checked = current == "true",
                            onCheckedChange = { values[field.id] = it.toString() },
                        )
                        Text(field.label)
                    }
                field.shape == "choice" -> {
                    Column {
                        Text(field.label, style = MaterialTheme.typography.labelMedium)
                        field.options.forEach { option ->
                            val selected = (field.selected ?: current) == option.value
                            TextButton(onClick = { values[field.id] = option.value }) {
                                Text((if (selected) "● " else "○ ") + option.label + (option.detail?.let { " · $it" } ?: ""))
                            }
                        }
                    }
                }
                else ->
                    OutlinedTextField(
                        value = current,
                        onValueChange = { values[field.id] = it },
                        label = { Text(field.label) },
                        modifier = Modifier.fillMaxWidth(),
                    )
            }
            field.hint?.let { Text(it, style = MaterialTheme.typography.labelSmall, color = MaterialTheme.colorScheme.onSurfaceVariant) }
        }
        if (submit != null) {
            Button(
                onClick = {
                    val filled =
                        shape.fields.map { field ->
                            FieldValue(field.id, field.label, values[field.id] ?: field.value)
                        }
                    onAction(node.id, submit, filled)
                    values.clear()
                },
            ) {
                Text(submit.label ?: "Send")
            }
        }
    }
}

@Composable
private fun ActionButton(node: String, action: Action, onAction: (String, Action, List<FieldValue>) -> Unit) {
    TextButton(onClick = { onAction(node, action, emptyList()) }) { Text(action.label ?: action.id) }
}

@Composable
private fun bodyFor(role: String) =
    when {
        role.startsWith("message.assistant") -> MaterialTheme.typography.bodyMedium
        role.startsWith("message.user") -> MaterialTheme.typography.bodyMedium
        role.startsWith("tool.") -> MaterialTheme.typography.bodySmall
        role.startsWith("message.system") -> MaterialTheme.typography.bodySmall.copy(fontStyle = FontStyle.Italic)
        else -> MaterialTheme.typography.bodyMedium
    }

/** The size a heading's level earns. The session decided the level; this decides the type. */
@Composable
private fun headingFor(level: Int) =
    when {
        level <= 1 -> MaterialTheme.typography.titleLarge
        level == 2 -> MaterialTheme.typography.titleMedium
        else -> MaterialTheme.typography.titleSmall
    }

@Composable
private fun roleSurface(role: String): Color? =
    when {
        role.startsWith("message.user") -> MaterialTheme.colorScheme.primaryContainer
        role.startsWith("tool.") -> MaterialTheme.colorScheme.surfaceVariant.copy(alpha = 0.5f)
        else -> null
    }

/** Inline runs, as one styled string. */
private fun inline(spans: List<Span>): AnnotatedString = buildAnnotatedString {
    spans.forEach { span ->
        when (span.kind) {
            "strong" -> withStyle(SpanStyle(fontWeight = FontWeight.Bold)) { append(span.text) }
            "emphasis" -> withStyle(SpanStyle(fontStyle = FontStyle.Italic)) { append(span.text) }
            "strikethrough" -> withStyle(SpanStyle(textDecoration = androidx.compose.ui.text.style.TextDecoration.LineThrough)) { append(span.text) }
            "code" -> withStyle(SpanStyle(fontFamily = FontFamily.Monospace)) { append(span.text) }
            "link" -> withStyle(SpanStyle(textDecoration = androidx.compose.ui.text.style.TextDecoration.Underline)) { append(span.text) }
            "token" -> withStyle(SpanStyle(color = tokenColor(span.text))) { append(span.text) }
            else -> append(span.text)
        }
    }
}

/** A code block with its captures painted. */
private fun highlight(shape: Shape.Code): AnnotatedString = buildAnnotatedString {
    var cursor = 0
    shape.captures
        .sortedBy { it.start }
        .forEach { capture ->
            val start = capture.start
            val end = capture.end.coerceAtMost(shape.text.length)
            if (start < cursor || start >= end) return@forEach
            append(shape.text.substring(cursor, start))
            withStyle(SpanStyle(color = tokenColor(capture.token))) { append(shape.text.substring(start, end)) }
            cursor = end
        }
    append(shape.text.substring(cursor.coerceAtMost(shape.text.length)))
}

/**
 * A capture name to a colour.
 *
 * Deliberately a small table: an unknown capture is plain text, because a
 * grammar that learned a token this frontend has not does not make a code block
 * unreadable. *Which* colours is the client's business; that a keyword is not a
 * string is the session's.
 */
private fun tokenColor(name: String): Color =
    when (name) {
        "keyword", "storage", "conditional", "repeat" -> Color(0xFFC678DD)
        "string", "char" -> Color(0xFF98C379)
        "function", "method" -> Color(0xFF61AFEF)
        "type", "class", "struct" -> Color(0xFFE5C07B)
        "number", "constant", "boolean" -> Color(0xFFD19A66)
        "comment" -> Color(0xFF7F848E)
        "operator", "punctuation" -> Color(0xFF56B6C2)
        else -> Color(0xFFABB2BF)
    }

@Composable
private fun Picture(shape: Shape.Picture) {
    if (shape.media.isNotEmpty() && !shape.media.startsWith("image/")) {
        Text(shape.alt, style = MaterialTheme.typography.bodySmall)
        return
    }
    val files = LocalImages.current
    val path = files.paths[shape.hash]
    LaunchedEffect(shape.hash, files.connected) { files.fetch(shape.hash) }
    val bitmap by produceState<android.graphics.Bitmap?>(null, path) {
        value = if (path == null) null else withContext(Dispatchers.IO) {
            runCatching {
                val options = BitmapFactory.Options().apply { inJustDecodeBounds = true }
                BitmapFactory.decodeFile(path, options)
                options.inSampleSize = 1
                while (options.outWidth / options.inSampleSize > 2048 || options.outHeight / options.inSampleSize > 2048) options.inSampleSize *= 2
                options.inJustDecodeBounds = false
                BitmapFactory.decodeFile(path, options)
            }.getOrNull()
        }
    }
    bitmap?.let { Image(it.asImageBitmap(), contentDescription = shape.alt, modifier = Modifier.fillMaxWidth()) }
        ?: Text(if (files.errors.containsKey(shape.hash) || path != null) "[image unavailable: ${shape.alt}]" else "[loading image: ${shape.alt}]", style = MaterialTheme.typography.bodySmall)
}

@Composable
private fun StreamText() {
    val display = LocalStreams.current
    display.streams.values.values.forEach { stream ->
        if (stream.text.isNotEmpty() && !display.contains(stream.id.substringBeforeLast('.', stream.id))) {
            androidx.compose.runtime.key(stream.id) { Text(stream.text, style = bodyFor(stream.role)) }
        }
    }
}
