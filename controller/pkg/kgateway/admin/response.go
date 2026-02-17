package admin

import (
	"encoding/json"
	"fmt"
)

// SnapshotResponseData is the data that is returned by Getter methods on the History object
// It allows us to encapsulate data and errors together, so that if an issue occurs during the request,
// we can get access to all the relevant information
type SnapshotResponseData struct {
	Data  any   `json:"data"`
	Error error `json:"error"`
}

// OutputFormat identifies the format to output an object
type OutputFormat int

const (
	// Json marshals the data into json, with indents included
	Json = iota

	// JsonCompact marshals the data into json, but without indents
	JsonCompact

	// Yaml marshals the data into yaml
	Yaml
)

func (f OutputFormat) String() string {
	return [...]string{"Json", "JsonCompact", "Yaml"}[f]
}

func (r SnapshotResponseData) Format(format OutputFormat) ([]byte, error) {
	// See: https://github.com/golang/go/issues/5161#issuecomment-1750037535
	var errorMsg string
	if r.Error != nil {
		errorMsg = r.Error.Error()
	}
	anon := struct {
		Data  any    `json:"data"`
		Error string `json:"error"`
	}{
		Data:  r.Data,
		Error: errorMsg,
	}

	return formatOutput(format, anon)
}

func (r SnapshotResponseData) MarshalJSON() ([]byte, error) {
	return r.Format(JsonCompact)
}

func (r SnapshotResponseData) MarshalJSONString() string {
	bytes, err := r.MarshalJSON()
	if err != nil {
		return err.Error()
	}
	return string(bytes)
}

// formatOutput formats a generic object into the specified output format
func formatOutput(format OutputFormat, genericOutput any) ([]byte, error) {
	switch format {
	case Json:
		return json.MarshalIndent(genericOutput, "", "    ")
	case JsonCompact:
		return json.Marshal(genericOutput)
	case Yaml:
		// There may be a case in the future, where yaml formatting is necessary
		// Since it is not required yet, we do not add support
		return nil, fmt.Errorf("%s format is not yet supported", format)
	default:
		return nil, fmt.Errorf("invalid format of %s", format)
	}
}
