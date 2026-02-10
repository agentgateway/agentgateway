package main

//go:generate ./generate.sh

func main() {
	panic("this file is a go:generate template and should not be included in the final build")
}
