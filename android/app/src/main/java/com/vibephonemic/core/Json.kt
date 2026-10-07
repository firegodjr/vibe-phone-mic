package com.vibephonemic.core

/** Tiny JSON helper for the flat objects used by the protocol (keeps the core free of org.json). */
object Json {
    fun obj(vararg pairs: Pair<String, Any?>): String =
        pairs.joinToString(separator = ",", prefix = "{", postfix = "}") { (k, v) -> "${quote(k)}:${value(v)}" }

    private fun value(v: Any?): String = when (v) {
        null -> "null"
        is String -> quote(v)
        is Boolean, is Int, is Long -> v.toString()
        is Number -> v.toString()
        else -> quote(v.toString())
    }

    fun quote(s: String): String {
        val sb = StringBuilder(s.length + 2).append('"')
        for (c in s) {
            when (c) {
                '"' -> sb.append("\\\"")
                '\\' -> sb.append("\\\\")
                '\n' -> sb.append("\\n")
                '\r' -> sb.append("\\r")
                '\t' -> sb.append("\\t")
                '\b' -> sb.append("\\b")
                '\u000C' -> sb.append("\\f")
                else -> if (c < ' ') sb.append(String.format("\\u%04x", c.code)) else sb.append(c)
            }
        }
        return sb.append('"').toString()
    }

    /** Parses a flat JSON object (string / number / boolean / null values). */
    fun parseFlat(text: String): Map<String, Any?> {
        val p = Parser(text)
        return p.parseObject()
    }

    private class Parser(val s: String) {
        var i = 0
        fun ws() { while (i < s.length && s[i].isWhitespace()) i++ }
        fun expect(c: Char) { ws(); require(i < s.length && s[i] == c) { "expected '$c' at $i" }; i++ }

        fun parseObject(): Map<String, Any?> {
            val out = LinkedHashMap<String, Any?>()
            expect('{'); ws()
            if (s[i] == '}') { i++; return out }
            while (true) {
                ws(); val key = parseString(); expect(':'); ws()
                out[key] = parseValue(); ws()
                if (s[i] == ',') { i++; continue }
                expect('}'); return out
            }
        }

        fun parseValue(): Any? = when {
            s[i] == '"' -> parseString()
            s.startsWith("true", i) -> { i += 4; true }
            s.startsWith("false", i) -> { i += 5; false }
            s.startsWith("null", i) -> { i += 4; null }
            else -> {
                val start = i
                while (i < s.length && (s[i].isDigit() || s[i] in "+-.eE")) i++
                val num = s.substring(start, i)
                num.toLongOrNull() ?: num.toDouble()
            }
        }

        fun parseString(): String {
            require(s[i] == '"') { "expected string at $i" }; i++
            val sb = StringBuilder()
            while (s[i] != '"') {
                val c = s[i++]
                if (c == '\\') {
                    when (val e = s[i++]) {
                        'n' -> sb.append('\n'); 'r' -> sb.append('\r'); 't' -> sb.append('\t')
                        'b' -> sb.append('\b'); 'f' -> sb.append('\u000C')
                        'u' -> { sb.append(s.substring(i, i + 4).toInt(16).toChar()); i += 4 }
                        else -> sb.append(e)
                    }
                } else sb.append(c)
            }
            i++
            return sb.toString()
        }
    }
}
