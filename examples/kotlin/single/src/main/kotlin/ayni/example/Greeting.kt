package ayni.example

fun greeting(name: String): String = "hello, ${name.trim().ifEmpty { "world" }}"

fun complexGreetingScore(input: String): Int {
    var score = 0
    for (char in input) {
        if (char.isUpperCase()) {
            score += 3
        } else if (char.isLowerCase()) {
            score += 2
        } else if (char.isDigit()) {
            score += 1
        } else if (char == '-') {
            score -= 1
        } else if (char == '_') {
            score -= 2
        } else {
            score += 0
        }
    }
    return when {
        score > 20 -> score * 2
        score > 10 -> score + 5
        score < 0 -> 0
        else -> score
    }
}
