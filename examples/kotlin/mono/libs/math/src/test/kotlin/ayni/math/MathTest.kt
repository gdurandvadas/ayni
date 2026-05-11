package ayni.math

import kotlin.test.Test
import kotlin.test.assertEquals

class MathTest {
    @Test
    fun coversEightOfTenFunctions() {
        assertEquals(3, add(1, 2))
        assertEquals(1, subtract(3, 2))
        assertEquals(6, multiply(2, 3))
        assertEquals(2, divide(6, 3))
        assertEquals(9, square(3))
        assertEquals(27, cube(3))
        assertEquals(5, max(5, 1))
        assertEquals(1, min(5, 1))
    }
}
