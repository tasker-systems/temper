# frozen_string_literal: true

require 'socket'

# Fork safety was the design spec's last "designed, not proven" thread. Proving
# it turned up something better than the design: the hazard is already handled
# one layer down.
#
# `net-http-persistent`'s pool subclasses `ConnectionPool`, and connection_pool
# >= 2.4 defaults `auto_reload_after_fork: true` -- it registers every pool in a
# WeakMap and, from a `Process._fork` hook, checks every connection back in and
# reloads the pool. So a forked child does NOT reuse its parent's pooled socket,
# whether or not it calls `Temper.reset_connection!`. Measured, not assumed: a
# child that skips the reset still causes a fresh TCP accept.
#
# `reset_connection!` remains the documented Puma/Sidekiq hook, because it clears
# OUR memo -- the ApiClient and its Faraday connection -- rather than leaning on a
# transitive dependency's default. These specs pin both halves: our memo
# semantics, and the no-socket-reuse behaviour we depend on. If connection_pool
# ever stops reloading after fork, the accept-count spec goes red here rather
# than silently in someone's Puma worker.
#
# The assertion is the server's ACCEPT COUNT, deliberately. Counting started
# `Net::HTTP` objects via ObjectSpace looked tempting and is wrong: the hook's
# `connection.close if connection.respond_to?(:close)` is a no-op for
# net-http-persistent (its Connection exposes `finish`/`reset`, not `close`), so
# the parent's socket object is dereferenced rather than finished -- and whether
# ObjectSpace still sees it depends on GC. An accept is observable and exact.
RSpec.describe 'fork safety' do
  before do
    skip 'fork is unsupported on this platform' unless Process.respond_to?(:fork)
    Temper.reset_connection!
    Temper.configure { |c| c.base_url = 'https://api.test' }
  end

  after { Temper.reset_connection! }

  # Run a block in a forked child; return the String it writes back.
  def in_child
    reader, writer = IO.pipe
    pid = fork do
      reader.close
      writer.write(yield.to_s)
      writer.close
      exit!(0)
    end
    writer.close
    result = reader.read
    reader.close
    Process.wait(pid)
    result
  end

  it 'gives a forked child a distinct connection after reset_connection!' do
    parent_id = Temper.api_client.object_id

    result = in_child do
      Temper.reset_connection!
      Temper.api_client.object_id == parent_id ? 'SHARED' : 'DISTINCT'
    end

    expect(result).to eq('DISTINCT')
  end

  # Without the reset, the child keeps the parent's memoized ApiClient object.
  # That is exactly what reset_connection! exists to clear.
  it 'a child that skips reset_connection! inherits the parent memo' do
    parent_id = Temper.api_client.object_id

    result = in_child { Temper.api_client.object_id == parent_id ? 'INHERITED' : 'FRESH' }

    expect(result).to eq('INHERITED')
  end

  # The load-bearing guarantee: the child inherits the memo but never reuses the
  # parent's pooled socket -- so it opens its own, and the server accepts again.
  #
  # Verified to bite: stub out ConnectionPool.after_fork and the child rides the
  # parent's socket, leaving the accept count at 1.
  it 'a forked child opens its own socket, even without reset_connection!' do
    server = keep_alive_server
    warm_the_connection(server[:port])
    expect(server[:accepts].size).to eq(1)

    result = in_child { child_request }

    shutdown(server)
    expect(result).to eq('OK')
    expect(server[:accepts].size).to eq(2)
  end

  it 'a forked child completes a real request after reset_connection!' do
    server = keep_alive_server
    warm_the_connection(server[:port])

    result = in_child do
      Temper.reset_connection!
      child_request
    end

    shutdown(server)
    expect(result).to eq('OK')
  end

  def bearer = Temper::Credentials::BearerToken.new('tok')

  def child_request
    Temper::Client.new(credentials: bearer).whoami
    'OK'
  rescue StandardError => e
    "ERR:#{e.class}: #{e.message}"
  end

  # Keep-alive, so the parent's warmed socket stays pooled and an inherited
  # connection WOULD be reusable if anything reused it.
  def keep_alive_server
    socket = TCPServer.new('127.0.0.1', 0)
    accepts = Queue.new
    thread = Thread.new do
      loop do
        client = socket.accept
        accepts << 1
        Thread.new(client) { |s| serve(s) }
      rescue IOError, Errno::EBADF
        break
      end
    end
    { socket: socket, port: socket.addr[1], accepts: accepts, thread: thread }
  end

  def serve(socket)
    loop do
      request_line = socket.gets
      break if request_line.nil?

      nil while (header = socket.gets) && header.strip != ''
      body = '{"id":"p1"}'
      socket.print("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n" \
                   "Content-Length: #{body.bytesize}\r\n\r\n#{body}")
    end
  rescue IOError, Errno::EPIPE, Errno::ECONNRESET
    nil
  end

  def warm_the_connection(port)
    WebMock.allow_net_connect!
    Temper.reset_connection!
    Temper.configure { |c| c.base_url = "http://127.0.0.1:#{port}" }
    Temper::Client.new(credentials: bearer).whoami
  end

  def shutdown(server)
    server[:thread].kill
    server[:socket].close
    WebMock.disable_net_connect!
  end
end
