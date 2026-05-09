package main
import (
 "fmt"
 "net/http"
 "go-mono/libs/greeting"
 "go-mono/libs/math"
 _ "github.com/sirupsen/logrus"
)
func main(){
 http.HandleFunc("/greet/", func(w http.ResponseWriter, r *http.Request){fmt.Fprintf(w, "%s Number=%d", greetinglib.Salute("dev"), mathlib.Square(3))})
 http.ListenAndServe(":8083", nil)
}
