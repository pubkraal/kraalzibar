PROTO_ROOT = ../../proto
GO_OUT = proto
MODULE = github.com/pubkraal/kraalzibar/sdks/go/proto/kraalzibar/v1

PROTOS = \
	kraalzibar/v1/core.proto \
	kraalzibar/v1/permission.proto \
	kraalzibar/v1/relationship.proto \
	kraalzibar/v1/schema.proto

M_OPTS = $(foreach p,$(PROTOS),M$(p)=$(MODULE))

.PHONY: proto test vet

proto:
	protoc \
		--proto_path=$(PROTO_ROOT) \
		--go_out=$(GO_OUT) --go_opt=paths=source_relative $(foreach m,$(M_OPTS),--go_opt=$(m)) \
		--go-grpc_out=$(GO_OUT) --go-grpc_opt=paths=source_relative $(foreach m,$(M_OPTS),--go-grpc_opt=$(m)) \
		$(PROTOS)

test:
	go test ./...

vet:
	go vet ./...
