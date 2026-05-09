package mathlib
import "testing"
func TestAdd(t *testing.T){ if Add(2,3)!=5 {t.Fail()} }
func TestSub(t *testing.T){ if Sub(5,2)!=3 {t.Fail()} }
func TestMul(t *testing.T){ if Mul(2,4)!=8 {t.Fail()} }
func TestDiv(t *testing.T){ if Div(8,2)!=4 {t.Fail()} }
func TestMod(t *testing.T){ if Mod(7,4)!=3 {t.Fail()} }
func TestSquare(t *testing.T){ if Square(3)!=9 {t.Fail()} }
func TestCube(t *testing.T){ if Cube(2)!=8 {t.Fail()} }
func TestMax(t *testing.T){ if Max(3,9)!=9 {t.Fail()} }
