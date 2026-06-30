package bedrock

import "strings"

// CRIS inference-profile geo prefixes, checked longest-first to avoid partial matches.
var crisPrefixes = []string{"us-gov.", "us.", "eu.", "apac.", "au.", "global.", "ca.", "sa.", "jp.", "il."}

func stripCRISPrefix(id string) string {
	for _, p := range crisPrefixes {
		if strings.HasPrefix(id, p) {
			return id[len(p):]
		}
	}
	return id
}
