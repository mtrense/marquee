// Package server serves.
package server

import "fmt"

// Server serves on a port.
type Server struct {
	Port int
}

// Handler handles requests.
type Handler interface {
	Handle(path string) error
}

// Start starts the server.
func (s *Server) Start() error {
	fmt.Println("listening on", s.Port)
	return nil
}

func (s Server) String() string {
	return fmt.Sprint(s.Port)
}

// New makes a server.
func New(port int) *Server {
	return &Server{Port: port}
}
