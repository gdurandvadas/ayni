package ayni.greeting

import kotlin.test.Test
import kotlin.test.assertEquals

class GreetingTest {
    @Test
    fun buildsGreeting() {
        assertEquals("hello, Ada", greeting("Ada"))
    }
}
