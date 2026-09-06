import kotlin.test.Test
import kotlin.test.assertEquals

class CounterTest {
    @Test
    fun increments() {
        assertEquals(3, increment(2))
    }
}
