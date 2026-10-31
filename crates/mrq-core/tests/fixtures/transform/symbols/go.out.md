# Go

```go
// Server serves on a port.
type Server struct {
	Port int
}
```

```go
type Handler interface {
	Handle(path string) error
}
```

```go
// Start starts the server.
func (s *Server) Start() error {
	fmt.Println("listening on", s.Port)
	return nil
}
```

```go
func (s Server) String() string {
	return fmt.Sprint(s.Port)
}
```

```golang
func New(port int) *Server {
	return &Server{Port: port}
}
```
