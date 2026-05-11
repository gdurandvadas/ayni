package ayni.example

import kotlin.test.Test
import kotlin.test.assertEquals

class GreetingTest {
    @Test
    fun greetsNamedUser() {
        assertEquals("hello, Ada", greeting("Ada"))
    }
}
