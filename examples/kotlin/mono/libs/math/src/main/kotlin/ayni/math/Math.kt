package ayni.math

fun add(left: Int, right: Int): Int = left + right
fun subtract(left: Int, right: Int): Int = left - right
fun multiply(left: Int, right: Int): Int = left * right
fun divide(left: Int, right: Int): Int = left / right
fun square(value: Int): Int = value * value
fun cube(value: Int): Int = value * value * value
fun max(left: Int, right: Int): Int = if (left > right) left else right
fun min(left: Int, right: Int): Int = if (left < right) left else right
fun increment(value: Int): Int = value + 1
fun decrement(value: Int): Int = value - 1
