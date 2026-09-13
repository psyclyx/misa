package org.misa.app

import org.json.JSONArray
import org.json.JSONObject

/**
 * The wire's own shapes, as data.
 *
 * Parsing is deliberately total: a shape this build has never heard of becomes
 * [Shape.Unknown] and is drawn as its children rather than dropped. A session
 * that learned a new node kind is a session this frontend should still be usable
 * with — the view vocabulary is open on purpose — and a client that threw on it
 * would turn a protocol addition into an outage.
 */
data class Span(val text: String, val kind: String, val href: String? = null)

data class Capture(val start: Int, val end: Int, val token: String)

data class Choice(val value: String, val label: String, val detail: String?)

data class Field(
    val id: String,
    val label: String,
    val value: String,
    val hint: String?,
    val shape: String,
    val options: List<Choice>,
    val selected: String?,
)

data class Action(val id: String, val on: String, val label: String?, val args: JSONObject?)

data class Node(
    val id: String,
    val role: String,
    val label: String?,
    val state: String?,
    val shape: Shape,
    val actions: List<Action>,
    val children: List<Node>,
) {
    /** The first action the node offers for a particular gesture. */
    fun actionOn(gesture: String): Action? = actions.firstOrNull { it.on == gesture }
}

sealed interface Shape {
    data object Section : Shape

    data class Text(val spans: List<Span>) : Shape

    /** A heading, and the level a document source gave it. */
    data class Heading(val level: Int, val spans: List<Span>) : Shape

    /** A block quotation; the blocks it quotes are the node's children. */
    data object Quote : Shape

    /** A thematic break. */
    data object Rule : Shape

    data class Code(val lang: String?, val text: String, val captures: List<Capture>) : Shape

    data class Bullets(val ordered: Boolean, val items: List<List<Node>>) : Shape

    data class Table(val head: List<List<Span>>, val rows: List<List<List<Span>>>) : Shape

    data class Fields(val fields: List<Field>) : Shape

    data class Collapsible(val summary: List<Span>, val open: Boolean) : Shape

    data class Picture(val alt: String) : Shape

    data class Status(val text: String) : Shape

    data class Meter(val meterLabel: String, val value: Double, val max: Double, val text: String) : Shape

    data class Fact(val value: Any?) : Shape

    data class Unknown(val wireShape: String) : Shape
}

object Wire {
    fun parseNode(json: JSONObject): Node {
        val actions = json.optJSONArray("actions")?.mapObjects { parseAction(it) } ?: emptyList()
        val children = json.optJSONArray("children")?.mapObjects { parseNode(it) } ?: emptyList()
        return Node(
            id = json.optString("id", ""),
            role = json.optString("role", "unknown"),
            label = json.optStringOrNull("label"),
            state = json.optStringOrNull("state"),
            shape = parseShape(json.optJSONObject("kind") ?: JSONObject()),
            actions = actions,
            children = children,
        )
    }

    private fun parseShape(kind: JSONObject): Shape {
        return when (val shape = kind.optString("shape", "section")) {
            "section" -> Shape.Section
            "text" -> Shape.Text(kind.optJSONArray("spans")?.mapObjects(::parseSpan) ?: emptyList())
            "heading" ->
                Shape.Heading(
                    level = kind.optInt("level", 1),
                    spans = kind.optJSONArray("spans")?.mapObjects(::parseSpan) ?: emptyList(),
                )
            "quote" -> Shape.Quote
            "rule" -> Shape.Rule
            "code" ->
                Shape.Code(
                    lang = kind.optStringOrNull("lang"),
                    text = kind.optString("text", ""),
                    captures =
                        kind.optJSONArray("captures")?.mapObjects {
                            Capture(it.optInt("start"), it.optInt("end"), it.optString("token", ""))
                        } ?: emptyList(),
                )
            "list" ->
                Shape.Bullets(
                    ordered = kind.optBoolean("ordered", false),
                    items =
                        kind.optJSONArray("items")?.mapArrays { item ->
                            item.mapObjects(::parseNode)
                        } ?: emptyList(),
                )
            "table" ->
                Shape.Table(
                    head = kind.optJSONArray("head")?.mapArrays { cell -> cell.mapObjects(::parseSpan) } ?: emptyList(),
                    rows =
                        kind.optJSONArray("rows")?.mapArrays { row ->
                            row.mapArrays { cell -> cell.mapObjects(::parseSpan) }
                        } ?: emptyList(),
                )
            "fields" -> Shape.Fields(kind.optJSONArray("fields")?.mapObjects(::parseField) ?: emptyList())
            "collapsible" ->
                Shape.Collapsible(
                    summary = kind.optJSONArray("summary")?.mapObjects(::parseSpan) ?: emptyList(),
                    open = kind.optBoolean("open", false),
                )
            "image" -> Shape.Picture(kind.optString("alt", ""))
            "status" -> Shape.Status(kind.optString("text", ""))
            "meter" ->
                Shape.Meter(
                    meterLabel = kind.optString("label", ""),
                    value = kind.optDouble("value", 0.0),
                    max = kind.optDouble("max", 100.0),
                    text = kind.optString("text", ""),
                )
            "fact" -> Shape.Fact(if (kind.has("value")) kind.get("value") else null)
            else -> Shape.Unknown(shape)
        }
    }

    private fun parseSpan(json: JSONObject): Span =
        Span(
            text = json.optString("text", ""),
            kind = json.optString("span", "plain"),
            href = json.optStringOrNull("href"),
        )

    private fun parseField(json: JSONObject): Field {
        val kind = json.optJSONObject("kind") ?: JSONObject()
        return Field(
            id = json.optString("id", ""),
            label = json.optString("label", ""),
            value = json.optString("value", ""),
            hint = json.optStringOrNull("hint"),
            shape = kind.optString("shape", "text"),
            options = kind.optJSONArray("options")?.mapObjects(::parseChoice) ?: emptyList(),
            selected = kind.optStringOrNull("selected"),
        )
    }

    private fun parseChoice(json: JSONObject): Choice =
        Choice(
            value = json.optString("value", ""),
            label = json.optString("label", ""),
            detail = json.optStringOrNull("detail"),
        )

    private fun parseAction(json: JSONObject): Action =
        Action(
            id = json.optString("id", ""),
            on = json.optString("on", "click"),
            label = json.optStringOrNull("label"),
            args = json.optJSONObject("args"),
        )

    fun parseSession(json: JSONObject): Session {
        val commands =
            json.optJSONArray("commands")?.mapObjects { command ->
                Command(
                    id = command.optString("id", ""),
                    label = command.optString("label", ""),
                    description = command.optString("description", ""),
                    args =
                        command.optJSONArray("args")?.mapObjects { arg ->
                            Argument(
                                name = arg.optString("name", ""),
                                label = arg.optString("label", ""),
                                required = arg.optBoolean("required", false),
                                source = arg.optStringOrNull("source"),
                            )
                        } ?: emptyList(),
                )
            } ?: emptyList()
        val sources =
            json.optJSONArray("sources")?.mapObjects { source ->
                Source(
                    id = source.optString("id", ""),
                    label = source.optString("label", ""),
                    onDemand = source.optString("kind", "resident") == "on_demand",
                )
            } ?: emptyList()
        return Session(
            id = json.optString("id", ""),
            title = json.optString("title", ""),
            commands = commands,
            sources = sources,
        )
    }
}

/** What a session says it is, and what it can be asked to do. */
data class Session(
    val id: String,
    val title: String,
    val commands: List<Command>,
    val sources: List<Source>,
)

data class Command(
    val id: String,
    val label: String,
    val description: String,
    val args: List<Argument>,
)

data class Argument(
    val name: String,
    val label: String,
    val required: Boolean,
    val source: String?,
)

data class Source(val id: String, val label: String, val onDemand: Boolean)

private fun JSONObject.optStringOrNull(key: String): String? {
    if (!has(key) || isNull(key)) return null
    val value = optString(key, "")
    return value.ifEmpty { null }
}

private inline fun <T> JSONArray.mapObjects(transform: (JSONObject) -> T): List<T> {
    val out = ArrayList<T>(length())
    for (index in 0 until length()) {
        optJSONObject(index)?.let { out.add(transform(it)) }
    }
    return out
}

private inline fun <T> JSONArray.mapArrays(transform: (JSONArray) -> T): List<T> {
    val out = ArrayList<T>(length())
    for (index in 0 until length()) {
        optJSONArray(index)?.let { out.add(transform(it)) }
    }
    return out
}
