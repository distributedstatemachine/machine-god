# Modern stdio fixture: the production-selected release helper must exec this
# process with pipes and persistent stdin. No external utilities or child jobs.
set -eu
umask 077
transcript=$1
discover=$2
list=$3
call=$4
printf 'started\n' >> "$transcript"
while IFS= read -r request; do
    printf '%s\n' "$request" >> "$transcript"
    [[ $request =~ \"id\":([0-9]+) ]] || exit 10
    request_id=${BASH_REMATCH[1]}
    case "$request" in
        *'"method":"server/discover"'*) result=$discover ;;
        *'"method":"tools/list"'*) result=$list ;;
        *'"method":"tools/call"'*) result=$call ;;
        *) exit 11 ;;
    esac
    printf '{"jsonrpc":"2.0","id":%s,"result":%s}\n' "$request_id" "$result"
done
printf 'eof\n' >> "$transcript"
