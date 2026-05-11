package ayni.service

import ayni.greeting.greeting
import ayni.math.increment

fun serviceGreeting(name: String): String {
    val count = increment(0)
    return "${greeting(name)} #$count"
}
