package org.misa.app

import androidx.compose.runtime.mutableStateMapOf
import org.json.JSONArray
import org.json.JSONObject

/** Canonical nodes are indexed; an operation replaces only its ancestor path. */
class ViewTree {
    private val nodes = mutableStateMapOf<String, Node>()
    private val parents = mutableMapOf<String, String>()
    var root: Node? = null
        private set
    var touched = 0
        private set
    fun contains(id: String): Boolean = nodes.containsKey(id)
    fun reset(tree: Node) {
        nodes.clear(); parents.clear(); root = tree; index(tree, null)
    }
    private fun index(node: Node, parent: String?) {
        check(nodes.put(node.id, node) == null) { "duplicate node ${node.id}" }
        if (parent != null) parents[node.id] = parent
        node.children.forEach { index(it, node.id) }
    }
    private fun erase(id: String) {
        val node = nodes.remove(id) ?: error("missing node $id")
        node.children.forEach { erase(it.id) }; parents.remove(id)
    }
    private fun update(node: Node) {
        touched++
        nodes[node.id] = node
        val parent = parents[node.id]
        if (parent == null) root = node
        else {
            val owner = nodes.getValue(parent)
            val at = owner.children.indexOfFirst { it.id == node.id }
            check(at >= 0)
            update(owner.copy(children = owner.children.toMutableList().also { it[at] = node }))
        }
    }
    fun apply(changes: JSONArray): Node {
        touched = 0
        for (i in 0 until changes.length()) {
            val ops = changes.getJSONObject(i).getJSONArray("ops")
            for (j in 0 until ops.length()) apply(ops.getJSONObject(j))
        }
        return requireNotNull(root)
    }
    private fun apply(op: JSONObject) {
        when (op.getString("op")) {
            "insert" -> {
                val parent = op.getString("parent")
                val owner = nodes.getValue(parent)
                val node = Wire.parseNode(op.getJSONObject("node"))
                val before = if (op.isNull("before")) null else op.optString("before").takeIf { it.isNotEmpty() }
                val at = before?.let { id -> owner.children.indexOfFirst { it.id == id }.also { check(it >= 0) } } ?: owner.children.size
                index(node, parent)
                update(owner.copy(children = owner.children.toMutableList().also { it.add(at, node) }))
            }
            "remove" -> {
                val id = op.getString("id")
                val parent = parents.getValue(id)
                val owner = nodes.getValue(parent)
                erase(id)
                update(owner.copy(children = owner.children.filterNot { it.id == id }))
            }
            "replace" -> {
                val id = op.getString("id")
                val parent = parents[id]
                val node = Wire.parseNode(op.getJSONObject("node"))
                check(id == node.id)
                erase(id); index(node, parent); update(node)
            }
            else -> error("unknown canonical operation")
        }
    }
}
