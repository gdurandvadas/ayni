package mathlib
func Add(a,b int) int { return a+b }
func Sub(a,b int) int { return a-b }
func Mul(a,b int) int { return a*b }
func Div(a,b int) int { return a/b }
func Mod(a,b int) int { return a%b }
func Square(a int) int { return a*a }
func Cube(a int) int { return a*a*a }
func Max(a,b int) int { if a>b {return a}; return b }
func Min(a,b int) int { if a<b {return a}; return b }
func Abs(a int) int { if a<0 {return -a}; return a }
