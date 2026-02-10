package cmdutils

import (
	"fmt"
	"strconv"
	"strings"
	"unicode"
)

// RunError represents an error running a Cmd
type RunError struct {
	command    []string // [Name Args...]
	output     []byte   // Captured Stdout / Stderr of the command
	inner      error    // Underlying error if any
	stackTrace error
}

var _ error = &RunError{}

func (e *RunError) Error() string {
	if e == nil {
		return ""
	}
	return fmt.Sprintf("command \"%s\" failed with error: %v", e.PrettyCommand(), e.inner)
}

// PrettyCommand pretty prints the command in a way that could be pasted
// into a shell
func (e *RunError) PrettyCommand() string {
	if e == nil {
		return "RunError is nil"
	}

	if len(e.command) == 0 {
		return "no command args"
	}

	if len(e.command) == 1 {
		return e.command[0]
	}

	// The above cases should not happen, but we defend against it
	return PrettyCommand(true, e.command[0], e.command[1:]...)
}

func (e *RunError) OutputString() string {
	if e == nil {
		return ""
	}
	return string(e.output)
}

// Cause mimics github.com/pkg/errors's Cause pattern for errors
func (e *RunError) Cause() error {
	if e == nil {
		return nil
	}
	return e.stackTrace
}

// PrettyCommand takes arguments identical to Cmder.Command, with a leading quote flag.
// Behavior:
// - quoteAll == true: quote all tokens (command and all args)
// - quoteAll == false: print tokens unquoted, but quote any arg that contains whitespace
// It returns a pretty printed command that could be pasted into a shell.
func PrettyCommand(quoteAll bool, name string, args ...string) string {
	var out strings.Builder
	if quoteAll {
		out.WriteString(strconv.Quote(name))
	} else {
		out.WriteString(name)
	}
	for _, arg := range args {
		out.WriteByte(' ')
		if quoteAll {
			out.WriteString(strconv.Quote(arg))
			continue
		}
		// Only quote arguments that contain whitespace when quote == false
		if strings.IndexFunc(arg, unicode.IsSpace) >= 0 {
			out.WriteString(strconv.Quote(arg))
			continue
		}
		out.WriteString(arg)
	}
	return out.String()
}
