package main
import (
 "fmt"
 "net/http"
)
func complexValue(n int) int { out:=0; for i:=0;i<n;i++{ if i%2==0 {out+=i} else if i%3==0 {out-=i} else if i%5==0 {out+=i*2} else if i%7==0 {out-=i*2} else {out++} }; return out }
func main(){
 http.HandleFunc("/greet/", func(w http.ResponseWriter, r *http.Request){fmt.Fprintf(w,"Hello! %d", complexValue(45))})
 http.ListenAndServe(":8082", nil)
}
