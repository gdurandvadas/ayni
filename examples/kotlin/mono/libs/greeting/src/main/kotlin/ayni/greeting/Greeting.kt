package ayni.greeting

import ayni.math.add

fun greeting(name: String): String = "hello, $name"

fun greetingCount(names: List<String>): Int = add(names.size, 0)
