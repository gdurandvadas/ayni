package ayni.service

import kotlin.test.Test
import kotlin.test.assertEquals

class ServiceTest {
    @Test
    fun buildsServiceGreeting() {
        assertEquals("hello, Ada #1", serviceGreeting("Ada"))
    }
}
